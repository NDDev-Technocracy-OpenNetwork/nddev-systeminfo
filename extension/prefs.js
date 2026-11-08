// A separate native GTK process. No Shell toolkit enters this process.
import Adw from 'gi://Adw';
import Gtk from 'gi://Gtk';
import Gio from 'gi://Gio';
import {ExtensionPreferences} from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';
import {Settings} from './lib/settings.js';

const LANGUAGES = [['system', 'system'], ['en', 'English'], ['ru', 'Русский'], ['kk', 'Қазақша'], ['ar', 'العربية'], ['zh', '中文']];
const INTERVALS = [[10, 'economy'], [5, 'standard'], [2, 'fast'], [0, 'realtime']];

export default class SysinfoPreferences extends ExtensionPreferences {
    fillPreferencesWindow(window) {
        const settings = new Settings(this.path);
        let page = null;
        let structure = null;
        const synchronize = [];
        const style = Adw.StyleManager.get_default();
        const provider = new Gtk.CssProvider();
        Gtk.StyleContext.add_provider_for_display(window.get_display(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION);
        const updateStyle = () => {
            const saved = settings.config.display.theme;
            const dark = saved.startsWith('dark') || saved === 'system' && style.dark;
            const contrast = style.high_contrast || saved.endsWith('-contrast');
            const c = settings.resource.themes[`${dark ? 'dark' : 'light'}${contrast ? '-contrast' : ''}`];
            const m = settings.resource.metrics[settings.config.display.density];
            provider.load_from_string(`.systeminfo-brand { background: ${c.surface}; color: ${c.accent}; border: ${m.border}px solid ${c.border}; border-radius: ${m.radius}px; padding: ${m.row_padding}px; }`);
        };
        const darkId = style.connect('notify::dark', updateStyle);
        const contrastId = style.connect('notify::high-contrast', updateStyle);
        const save = (key, value) => {
            void settings.set(key, value)
                .then(message => window.add_toast(new Adw.Toast({title: settings.text(message)})))
                .catch(() => window.add_toast(new Adw.Toast({title: settings.text('error')})));
        };
        const combo = (group, label, options, read, changed) => {
            const row = new Adw.ComboRow({title: settings.text(label)});
            let choices = [];
            let selected;
            let syncing = false;
            const sync = () => {
                syncing = true;
                selected = read();
                const next = typeof options === 'function' ? options() : options;
                if (JSON.stringify(next) !== JSON.stringify(choices)) {
                    choices = next;
                    const model = new Gtk.StringList();
                    for (const [, text] of choices) model.append(text);
                    row.model = model;
                }
                row.selected = choices.findIndex(([value]) => value === selected);
                syncing = false;
            };
            sync(); synchronize.push(sync);
            row.connect('notify::selected', () => {
                const value = choices[row.selected]?.[0];
                if (!syncing && value !== undefined && value !== selected) changed(value);
            });
            group.add(row);
        };
        const rebuild = () => {
            const c = settings.config;
            window.title = `${settings.text('app_title')} · ${settings.text('settings')}`;
            Gtk.Widget.set_default_direction(settings.rtl ? Gtk.TextDirection.RTL : Gtk.TextDirection.LTR);
            style.color_scheme = c.display.theme.startsWith('dark') ? Adw.ColorScheme.FORCE_DARK : c.display.theme.startsWith('light') ? Adw.ColorScheme.FORCE_LIGHT : Adw.ColorScheme.DEFAULT;
            updateStyle();
            const nextStructure = JSON.stringify([settings.ready, settings.error, c.display.locale]);
            if (page && nextStructure === structure) {
                for (const sync of synchronize) sync();
                return;
            }
            structure = nextStructure;
            if (page) window.remove(page);
            synchronize.length = 0;
            page = new Adw.PreferencesPage();
            const brand = new Adw.PreferencesGroup();
            const heading = new Adw.ActionRow({title: settings.text('app_title'), subtitle: settings.resource.version});
            heading.add_css_class('systeminfo-brand');
            const icon = new Gtk.Image({gicon: new Gio.FileIcon({file: Gio.File.new_for_path(`${this.path}/resources/brand-symbolic.svg`)}), pixel_size: settings.resource.metrics[c.display.density].icon});
            heading.add_prefix(icon);
            synchronize.push(() => { icon.pixel_size = settings.resource.metrics[settings.config.display.density].icon; });
            brand.add(heading); page.add(brand);
            if (!settings.ready) {
                const state = new Adw.PreferencesGroup();
                state.add(new Adw.ActionRow({title: settings.text(settings.error ? 'error' : 'loading'), subtitle: settings.error ? settings.text('provider_help') : ''}));
                page.add(state); window.add(page); return;
            }
            const reading = new Adw.PreferencesGroup({title: settings.text('readings'), description: settings.text('interval_description')});
            const intervals = () => {
                const values = INTERVALS.map(([seconds, key]) => [seconds, settings.text(key)]);
                const seconds = settings.config.sampling.interval_seconds;
                if (!values.some(([value]) => value === seconds)) values.push([seconds, settings.text('custom_interval').replace('{seconds}', String(seconds))]);
                return values;
            };
            combo(reading, 'interval', intervals, () => settings.config.sampling.interval_seconds, value => save('sampling.interval_seconds', value));
            const gpu = new Adw.SwitchRow({title: settings.text('collect_gpu'), subtitle: settings.text('gpu_description'), active: c.sampling.gpu});
            synchronize.push(() => { gpu.active = settings.config.sampling.gpu; });
            gpu.connect('notify::active', () => { if (gpu.active !== settings.config.sampling.gpu) save('sampling.gpu', gpu.active); });
            reading.add(gpu);
            page.add(reading);
            const panel = new Adw.PreferencesGroup({title: settings.text('panel')});
            for (const key of ['show_cpu', 'show_memory', 'show_gpu', 'show_network', 'show_temperatures']) {
                const row = new Adw.SwitchRow({title: settings.text(key), active: c.display[key]});
                synchronize.push(() => { row.active = settings.config.display[key]; });
                row.connect('notify::active', () => { if (row.active !== settings.config.display[key]) save(`display.${key}`, row.active); });
                panel.add(row);
            }
            page.add(panel);
            const appearance = new Adw.PreferencesGroup({title: settings.text('appearance')});
            combo(appearance, 'theme', ['system', ...Object.keys(settings.resource.themes)].map(key => [key, settings.text(key)]), () => settings.config.display.theme, value => save('display.theme', value));
            combo(appearance, 'density', ['compact', 'comfortable'].map(key => [key, settings.text(key)]), () => settings.config.display.density, value => save('display.density', value));
            combo(appearance, 'language', LANGUAGES.map(([value, title]) => [value, value === 'system' ? settings.text(value) : title]), () => settings.config.display.locale, value => save('display.locale', value));
            page.add(appearance);
            if (settings.error) {
                const error = new Adw.PreferencesGroup({title: settings.text('error')});
                error.add(new Adw.ActionRow({title: settings.text('provider_help')})); page.add(error);
            }
            window.add(page);
        };
        settings.connect(rebuild);
        window.connect('close-request', () => { settings.close(); style.disconnect(darkId); style.disconnect(contrastId); Gtk.StyleContext.remove_provider_for_display(window.get_display(), provider); return false; });
        rebuild(); void settings.start();
    }
}
