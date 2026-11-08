import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import {Client} from '../lib/client.js';

const path = GLib.build_filenamev([GLib.get_user_runtime_dir(), 'rldyour-sysinfo.sock']);
const listener = new Gio.SocketListener();
listener.add_address(new Gio.UnixSocketAddress({path}), Gio.SocketType.STREAM,
    Gio.SocketProtocol.DEFAULT, null);
const loop = new GLib.MainLoop(null, false);
let client;
let connection;
let samples = 0;
let states = 0;
let failure = null;
const frame = JSON.stringify({v: 1, cpu: {usage: null, temp: null}, memory: {used: null, swap: null}, gpu: {usage: null, memory: null, temp: null}, disk: {read: null, write: null, temp: null}, net: {rx: null, tx: null}});
let timeout = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 5, () => {
    timeout = 0;
    failure = 'client timed out';
    loop.quit();
    return GLib.SOURCE_REMOVE;
});

listener.accept_async(null, (source, result) => {
    [connection] = source.accept_finish(result);
    const output = connection.get_output_stream();
    // One partial frame, then its tail and a second frame in a single read.
    const first = new TextEncoder().encode(frame.slice(0, 5));
    output.write_all(first, null);
    GLib.idle_add(GLib.PRIORITY_DEFAULT, () => {
        output.write_all(new TextEncoder().encode(`${frame.slice(5)}\n${frame}\n`), null);
        return GLib.SOURCE_REMOVE;
    });
});

client = new Client(5, sample => {
    if (sample.v !== 1)
        failure = 'invalid sample';
    samples++;
    if (samples === 2) {
        client.stop(); // A consumer may stop during the read callback.
        loop.quit();
    }
}, connected => { if (connected) states++; }, {client_timeout_ms: 2000, reconnect_seconds: 1});
loop.run();
client.stop();
connection?.close(null);
listener.close();
if (timeout)
    GLib.Source.remove(timeout);
Gio.File.new_for_path(path).delete(null);
if (failure || samples !== 2 || states !== 1)
    throw new Error(failure ?? `samples=${samples}, states=${states}`);
print('PASS: asynchronous handshake, fragmented/coalesced frames, stop during callback');
