/* rldyour-sysinfo — preferences
 *
 * Runs in its own process, which is why nothing here touches St, Clutter, Meta
 * or Shell: those libraries exist only inside gnome-shell.
 */

import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import Gtk from 'gi://Gtk';

import {ExtensionPreferences} from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';

/** Panel toggles, in the order they appear in the panel itself. */
const TOGGLES = [
    {key: 'show-cpu', title: 'Processor', subtitle: 'Busy share of all cores'},
    {key: 'show-memory', title: 'Memory', subtitle: 'Share unavailable to new allocations'},
    {key: 'show-gpu', title: 'Graphics', subtitle: 'Requires an NVIDIA driver'},
    {key: 'show-network', title: 'Network', subtitle: 'Throughput across physical interfaces'},
    {key: 'show-temperatures', title: 'Temperatures', subtitle: 'Shown next to processor and graphics'},
];

/**
 * Cadence presets, ordered slowest to fastest. `0` is the wire value for the
 * daemon's realtime mode — half-second ticks — not "off". A ComboRow shows
 * only the title, so each label carries its own timing.
 */
const MODES = [
    {seconds: 10, title: 'Economy — every 10 seconds'},
    {seconds: 5, title: 'Standard — every 5 seconds'},
    {seconds: 2, title: 'Fast — every 2 seconds'},
    {seconds: 0, title: 'Realtime — twice a second'},
];

export default class SysinfoPreferences extends ExtensionPreferences {
    fillPreferencesWindow(window) {
        const settings = this.getSettings();
        const page = new Adw.PreferencesPage();

        const reading = new Adw.PreferencesGroup({
            title: 'Readings',
            description: 'The daemon serves the fastest cadence any client asks for.',
        });
        const interval = new Adw.ComboRow({
            title: 'Interval',
            subtitle: 'How often the daemon publishes a reading',
        });
        const titles = MODES.map(mode => mode.title);
        const values = MODES.map(mode => mode.seconds);
        // A hand-set gsettings value survives: offer it rather than silently
        // snapping the display to the nearest preset.
        const stored = settings.get_int('interval');
        if (!values.includes(stored)) {
            values.push(stored);
            titles.push(`Custom — every ${stored} seconds`);
        }
        const list = new Gtk.StringList();
        for (const title of titles)
            list.append(title);
        interval.model = list;
        interval.selected = values.indexOf(stored);
        interval.connect('notify::selected', row => {
            const seconds = values[row.selected];
            if (seconds !== undefined && settings.get_int('interval') !== seconds)
                settings.set_int('interval', seconds);
        });
        settings.connect('changed::interval', () => {
            const index = values.indexOf(settings.get_int('interval'));
            if (index !== -1 && interval.selected !== index)
                interval.selected = index;
        });
        reading.add(interval);
        page.add(reading);

        const panel = new Adw.PreferencesGroup({
            title: 'Panel',
            description: 'What appears in the top bar. The menu always lists everything.',
        });
        for (const toggle of TOGGLES) {
            const row = new Adw.SwitchRow({title: toggle.title, subtitle: toggle.subtitle});
            settings.bind(toggle.key, row, 'active', Gio.SettingsBindFlags.DEFAULT);
            panel.add(row);
        }
        page.add(panel);

        window.add(page);
    }
}
