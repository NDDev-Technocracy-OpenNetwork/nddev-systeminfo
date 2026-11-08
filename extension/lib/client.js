// One bounded stream, no polling. Deadlines exist only while awaiting data.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import {decode, MAX_LINE, MAX_INTERVAL, REALTIME_MS} from './protocol.js';

export class Client {
    constructor(interval, onSample, onState, transport) {
        if (!Number.isInteger(interval) || interval < 0 || interval > MAX_INTERVAL)
            throw new Error('Invalid Systeminfo interval');
        this._interval = interval; this._onSample = onSample; this._onState = onState;
        this._timeoutMs = transport.client_timeout_ms; this._retrySeconds = transport.reconnect_seconds;
        this._cancellable = new Gio.Cancellable(); this._connection = null; this._stream = null;
        this._reconnectId = 0; this._deadlineId = 0; this._generation = 0; this._connected = false;
        this._buffer = new Uint8Array(MAX_LINE); this._used = 0;
        this._decoder = new TextDecoder('utf-8', {fatal: true});
        this._connect();
    }
    _current(generation) { return generation === this._generation && !this._cancellable.is_cancelled(); }
    _connect() {
        const generation = ++this._generation;
        const path = GLib.build_filenamev([GLib.get_user_runtime_dir(), 'rldyour-sysinfo.sock']);
        const client = new Gio.SocketClient({timeout: Math.max(1, Math.ceil(this._timeoutMs / 1000))});
        this._deadline(generation, this._timeoutMs);
        client.connect_async(new Gio.UnixSocketAddress({path}), this._cancellable, (source, result) => {
            let connection;
            try { connection = source.connect_finish(result); }
            catch { if (this._current(generation)) this._retry(generation); return; }
            if (!this._current(generation)) { connection.close_async(GLib.PRIORITY_DEFAULT, null, null); return; }
            // The GLib absolute deadline includes the requested cadence. A
            // connect timeout must not become a shorter per-read timeout.
            connection.get_socket().set_timeout(0);
            this._connection = connection; this._stream = connection.get_input_stream();
            const bytes = new TextEncoder().encode(`{"interval":${this._interval}}\n`);
            connection.get_output_stream().write_all_async(bytes, GLib.PRIORITY_DEFAULT, this._cancellable, (output, written) => {
                try { output.write_all_finish(written); }
                catch { if (this._current(generation)) this._retry(generation); }
            });
            this._deadline(generation, this._timeoutMs + (this._interval === 0 ? REALTIME_MS : this._interval * 1000));
            this._read(generation);
        });
    }
    _deadline(generation, milliseconds) {
        if (this._deadlineId) GLib.Source.remove(this._deadlineId);
        this._deadlineId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, milliseconds, () => {
            this._deadlineId = 0;
            if (this._current(generation)) this._retry(generation);
            return GLib.SOURCE_REMOVE;
        });
    }
    _read(generation) {
        this._stream.read_bytes_async(MAX_LINE, GLib.PRIORITY_DEFAULT, this._cancellable, (source, result) => {
            let bytes;
            try { bytes = source.read_bytes_finish(result).get_data(); }
            catch { if (this._current(generation)) this._retry(generation); return; }
            if (!this._current(generation)) return;
            if (bytes.length === 0) { this._retry(generation); return; }
            try {
                let start = 0;
                while (start < bytes.length) {
                    const newline = bytes.indexOf(10, start);
                    const end = newline === -1 ? bytes.length : newline;
                    if (this._used + end - start > MAX_LINE) throw new Error('Oversized Systeminfo frame');
                    this._buffer.set(bytes.subarray(start, end), this._used); this._used += end - start;
                    if (newline === -1) break;
                    const sample = decode(this._decoder.decode(this._buffer.subarray(0, this._used)));
                    this._used = 0;
                    this._deadline(generation, this._timeoutMs + (this._interval === 0 ? REALTIME_MS : this._interval * 1000));
                    if (!this._connected) { this._connected = true; this._onState(true); }
                    if (!this._current(generation)) return;
                    this._onSample(sample);
                    if (!this._current(generation)) return;
                    start = end + 1;
                }
            } catch { this._retry(generation); return; }
            if (this._current(generation)) this._read(generation);
        });
    }
    _retry(generation) {
        if (!this._current(generation)) return;
        this._closeConnection(); this._onState(false);
        if (this._cancellable.is_cancelled()) return;
        if (this._reconnectId) GLib.Source.remove(this._reconnectId);
        this._reconnectId = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, this._retrySeconds, () => {
            this._reconnectId = 0; this._connect(); return GLib.SOURCE_REMOVE;
        });
    }
    _closeConnection() {
        ++this._generation;
        if (this._deadlineId) GLib.Source.remove(this._deadlineId);
        this._deadlineId = 0;
        this._connection?.close_async(GLib.PRIORITY_DEFAULT, null, null);
        this._connection = null; this._stream = null; this._used = 0; this._connected = false;
    }
    stop() {
        if (this._reconnectId) GLib.Source.remove(this._reconnectId);
        this._reconnectId = 0; this._cancellable.cancel(); this._closeConnection();
    }
}
