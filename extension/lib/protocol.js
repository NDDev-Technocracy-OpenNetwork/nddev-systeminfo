// Shape and limits are generated from the same schema as the Rust contract.
import {METRICS, PROTOCOL_VERSION} from './contract.js';
export {MAX_LINE, MAX_INTERVAL, REALTIME_MS} from './contract.js';

export function decode(line) {
    const sample = JSON.parse(line);
    if (sample === null || typeof sample !== 'object' || sample.v !== PROTOCOL_VERSION)
        throw new Error('Unsupported Systeminfo frame');
    for (const [group, fields] of Object.entries(METRICS)) {
        const values = sample[group];
        if (values === null || typeof values !== 'object' || Array.isArray(values))
            throw new Error('Missing Systeminfo group');
        for (const [key, kind] of Object.entries(fields)) {
            if (!Object.hasOwn(values, key)) throw new Error('Missing Systeminfo metric');
            const value = values[key];
            if (value === null) continue;
            if (typeof value !== 'number' || !Number.isFinite(value)
                || kind === 'integer' && (!Number.isInteger(value) || value < 0 || value > 18446744073709551615)
                || kind === 'percent' && (value < 0 || value > 100))
                throw new Error('Invalid Systeminfo metric');
        }
    }
    return sample;
}
