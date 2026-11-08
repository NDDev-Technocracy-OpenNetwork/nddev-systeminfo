import {decode, MAX_LINE} from '../lib/protocol.js';
import {percent, rate, celsius, ABSENT} from '../lib/format.js';

const sample = {v: 1, cpu: {usage: null, temp: null}, memory: {used: null, swap: null}, gpu: {usage: null, memory: null, temp: null}, disk: {read: null, write: null, temp: null}, net: {rx: null, tx: null}};
const assert = (condition, message) => { if (!condition) throw new Error(message); };
assert(decode(JSON.stringify(sample)).v === 1, 'nullable frame rejected');
for (const [group, fields] of Object.entries(sample)) {
    if (group === 'v') continue;
    for (const key of Object.keys(fields)) {
        const invalid = JSON.parse(JSON.stringify(sample)); delete invalid[group][key];
        let rejected = false; try { decode(JSON.stringify(invalid)); } catch { rejected = true; }
        assert(rejected, `missing ${group}.${key} accepted`);
    }
}
for (const value of [-1, 101, '5', true]) {
    const invalid = JSON.parse(JSON.stringify(sample)); invalid.cpu.usage = value;
    let rejected = false; try { decode(JSON.stringify(invalid)); } catch { rejected = true; }
    assert(rejected, 'invalid percentage accepted');
}
sample.future = true;
assert(decode(JSON.stringify(sample)).future, 'compatible extension rejected');
assert(MAX_LINE > JSON.stringify(sample).length, 'normal frame exceeds the contract ceiling');
assert(percent(Infinity) === ABSENT && percent(-1) === ABSENT, 'invalid percentages displayed');
assert(celsius(null) === ABSENT && rate(-1) === ABSENT, 'unavailable values displayed as numbers');
assert(rate(1024) === '1.0 KiB/s', 'throughput unit is ambiguous');
print('PASS: native UI wire validation, unavailable values and IEC rate units');
