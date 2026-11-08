/* rldyour-sysinfo — companion daemon client
 */

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

const SOCKET_NAME = 'rldyour-sysinfo.sock';
const RECONNECT_SECONDS = 5;
/** Refuse a line long enough to mean the peer is not our daemon. */
const MAX_LINE = 4096;

/**
 * Reads newline-delimited samples from the daemon socket.
 *
 * The cadence belongs entirely to the daemon: this client never runs a timer
 * while it is connected, it only waits on the next line. The single timer it
 * can own is the reconnect backoff, which exists because the daemon is allowed
 * to exit when nobody is watching.
 */
export class Client {
    /**
     * @param {number} interval seconds between readings to ask for; 0 is realtime
     * @param {(sample: object) => void} onSample called for each decoded line
     * @param {(connected: boolean) => void} onState called when the link changes
     */
    constructor(interval, onSample, onState) {
        this._interval = interval;
        this._onSample = onSample;
        this._onState = onState;
        this._cancellable = new Gio.Cancellable();
        this._connection = null;
        this._stream = null;
        this._reconnectId = 0;
        this._buffer = new Uint8Array(0);
        this._decoder = new TextDecoder('utf-8', {fatal: true});
        this._connect();
    }

    _connect() {
        const path = GLib.build_filenamev([GLib.get_user_runtime_dir(), SOCKET_NAME]);
        const client = new Gio.SocketClient();

        client.connect_async(new Gio.UnixSocketAddress({path}), this._cancellable, (source, result) => {
            let connection;
            try {
                connection = source.connect_finish(result);
            } catch (error) {
                if (!error.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED))
                    this._retry();
                return;
            }

            if (this._cancellable.is_cancelled()) {
                connection.close_async(GLib.PRIORITY_DEFAULT, null, null);
                return;
            }

            this._connection = connection;
            this._stream = connection.get_input_stream();
            this._announce(connection);
            this._onState(true);
            this._read();
        });
    }

    /**
     * States the cadence this client wants. The daemon serves the fastest any
     * client asked for, and treats silence as "no preference", so a failure
     * here costs the requested interval and nothing else.
     */
    _announce(connection) {
        try {
            const bytes = new TextEncoder().encode(`{"interval":${this._interval}}\n`);
            connection.get_output_stream().write_all_async(bytes, GLib.PRIORITY_DEFAULT, this._cancellable, (source, result) => {
                try {
                    source.write_all_finish(result);
                } catch (error) {
                    if (!this._cancellable.is_cancelled())
                        this._retry();
                }
            });
        } catch {
            // Falls back to whatever the daemon is configured for.
        }
    }

    _read() {
        this._stream.read_bytes_async(MAX_LINE, GLib.PRIORITY_DEFAULT, this._cancellable, (source, result) => {
            let bytes;
            try {
                bytes = source.read_bytes_finish(result).get_data();
            } catch (error) {
                if (!error.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED))
                    this._retry();
                return;
            }

            // A null line is a clean close, which happens whenever the daemon
            // decides nobody needed it any more.
            if (this._cancellable.is_cancelled())
                return;
            if (bytes.length === 0) {
                this._retry();
                return;
            }

            const combined = new Uint8Array(this._buffer.length + bytes.length);
            combined.set(this._buffer);
            combined.set(bytes, this._buffer.length);
            let start = 0;
            for (let end = 0; end < combined.length; end++) {
                if (combined[end] !== 10)
                    continue;
                if (end - start > MAX_LINE) {
                    this._retry();
                    return;
                }
                try {
                    this._decode(this._decoder.decode(combined.subarray(start, end)));
                } catch {
                    this._retry();
                    return;
                }
                if (this._stream === null)
                    return;
                start = end + 1;
            }
            this._buffer = combined.slice(start);
            if (this._buffer.length > MAX_LINE) {
                this._retry();
                return;
            }

            // The consumer is allowed to stop us from inside the callback
            // above, which is exactly what the indicator does when the shell
            // disables the extension. That tears the stream down, so there is
            // nothing left to continue reading from.
            if (this._stream !== null)
                this._read();
        });
    }

    _decode(line) {
        let sample;
        try {
            sample = JSON.parse(line);
        } catch {
            // A malformed line means a version skew or a foreign peer. Skipping
            // it keeps the indicator on its last good reading.
            return;
        }
        // The version byte is the protocol contract: anything else is a
        // different daemon or a future format this client cannot read.
        if (sample?.v !== 1)
            return;
        this._onSample(sample);
    }

    _retry() {
        this._closeConnection();
        if (this._cancellable.is_cancelled())
            return;
        this._onState(false);

        if (this._reconnectId)
            GLib.Source.remove(this._reconnectId);
        this._reconnectId = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, RECONNECT_SECONDS, () => {
            this._reconnectId = 0;
            this._connect();
            return GLib.SOURCE_REMOVE;
        });
    }

    _closeConnection() {
        if (this._connection)
            this._connection.close_async(GLib.PRIORITY_DEFAULT, null, null);
        this._connection = null;
        this._stream = null;
        this._buffer = new Uint8Array(0);
    }

    /** Releases the socket and the reconnect timer. The client is done after this. */
    stop() {
        if (this._reconnectId)
            GLib.Source.remove(this._reconnectId);
        this._reconnectId = 0;

        this._cancellable.cancel();
        this._closeConnection();
    }
}
