/* rldyour-sysinfo — value formatting
 *
 * Imports nothing from the shell toolkit, so the same helpers stay usable from
 * a preferences process, which may not load St or Clutter.
 */

/** Shown wherever the host cannot supply a metric. */
export const ABSENT = '—';

const RATE_UNITS = ['B/s', 'KiB/s', 'MiB/s', 'GiB/s', 'TiB/s', 'PiB/s', 'EiB/s'];
const RATE_STEP = 1024;

function missing(value) {
    return typeof value !== 'number' || !Number.isFinite(value);
}

export function percent(value) {
    return missing(value) || value < 0 || value > 100 ? ABSENT : `${Math.round(value)}%`;
}

export function celsius(value) {
    return missing(value) ? ABSENT : `${Math.round(value)} °C`;
}

/**
 * Formats a per-second byte count for a panel, where width matters more than
 * precision: one fractional digit only while the mantissa is small enough for
 * it to carry information.
 */
export function rate(value) {
    if (missing(value) || value < 0)
        return ABSENT;

    let scaled = value;
    let unit = 0;
    while (scaled >= RATE_STEP && unit < RATE_UNITS.length - 1) {
        scaled /= RATE_STEP;
        unit += 1;
    }

    const digits = unit > 0 && scaled < 10 ? 1 : 0;
    return `${scaled.toFixed(digits)} ${RATE_UNITS[unit]}`;
}

/** Directional variants for the panel, where the caption cannot say which is
 * which. Arrows, not emoji: the shell renders these in the panel font and they
 * stay legible at the size a top bar actually uses. */
export function rateIn(value) {
    return `\u2193${rate(value)}`;
}

export function rateOut(value) {
    return `\u2191${rate(value)}`;
}
