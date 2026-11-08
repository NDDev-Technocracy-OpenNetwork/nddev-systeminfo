/* rldyour-sysinfo — smoke test
 *
 * Exercises the two modules that run outside GNOME Shell: value formatting,
 * and the socket client against a live daemon. Run it from this directory:
 *
 *     gjs -m tests/smoke.js
 *
 * It needs the daemon reachable, which socket activation arranges on connect.
 * A JS ERROR in the output is a failure even when the tally says otherwise.
 */

import GLib from 'gi://GLib';
import {Client} from '../lib/client.js';
import {ABSENT, celsius, percent, rate} from '../lib/format.js';

let failures = 0;
function check(actual, expected, label) {
    if (actual !== expected) { print(`  ✗ ${label}: got "${actual}", expected "${expected}"`); failures++; }
    else print(`  ✓ ${label} = ${actual}`);
}

print('=== format ===');
check(percent(55.4), '55%', 'percent(55.4)');
check(percent(null), ABSENT, 'percent(null)');
check(celsius(88.6), '89 °C', 'celsius(88.6)');
check(rate(0), '0 B/s', 'rate(0)');
check(rate(7991), '7.8 KiB/s', 'rate(7991)');
check(rate(342578), '335 KiB/s', 'rate(342578)');
check(rate(2 * 1024 * 1024 * 1024), '2.0 GiB/s', 'rate(2GiB)');
check(rate(null), ABSENT, 'rate(null)');

print('\n=== client against a live daemon ===');
const loop = GLib.MainLoop.new(null, false);
let samples = 0;

const REQUESTED_INTERVAL = 2;
const client = new Client(REQUESTED_INTERVAL, sample => {
    samples++;
    print(`  [${samples}] v=${sample.v} cpu=${percent(sample.cpu.usage)}/${celsius(sample.cpu.temp)} ` +
          `ram=${percent(sample.memory.used)} gpu=${percent(sample.gpu.usage)}/${celsius(sample.gpu.temp)} ` +
          `net=↓${rate(sample.net.rx)} ↑${rate(sample.net.tx)}`);
    if (samples >= 2) { client.stop(); loop.quit(); }
}, connected => print(`  connection: ${connected ? 'up' : 'lost'}`), {client_timeout_ms: 5000, reconnect_seconds: 1});

GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 25, () => {
    print('  ✗ timed out waiting for samples'); failures++; client.stop(); loop.quit();
    return GLib.SOURCE_REMOVE;
});

loop.run();
print(`\nresult: ${failures === 0 ? 'all green' : failures + ' failure(s)'}`);
if (failures > 0) imports.system.exit(1);
