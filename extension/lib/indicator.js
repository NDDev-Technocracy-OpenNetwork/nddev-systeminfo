import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GObject from 'gi://GObject';
import St from 'gi://St';
import Pango from 'gi://Pango';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Client} from './client.js';
import {ABSENT, celsius, percent, rate, rateIn, rateOut} from './format.js';

const CELLS = [
    {setting: 'show_cpu', caption: 'CPU', fields: ['cpu.usage:percent'], temperature: 'cpu.temp:celsius'},
    {setting: 'show_memory', caption: 'RAM', fields: ['memory.used:percent']},
    {setting: 'show_gpu', caption: 'GPU', fields: ['gpu.usage:percent'], temperature: 'gpu.temp:celsius'},
    {setting: 'show_network', caption: '↔', fields: ['net.rx:rateIn', 'net.tx:rateOut']},
];
const ROWS = [
    ['cpu', 'cpu.usage:percent'], ['cpu_temperature', 'cpu.temp:celsius'],
    ['memory', 'memory.used:percent'], ['swap', 'memory.swap:percent'],
    ['gpu', 'gpu.usage:percent'], ['gpu_memory', 'gpu.memory:percent'], ['gpu_temperature', 'gpu.temp:celsius'],
    ['disk_read', 'disk.read:rate'], ['disk_write', 'disk.write:rate'], ['disk_temperature', 'disk.temp:celsius'],
    ['network_rx', 'net.rx:rate'], ['network_tx', 'net.tx:rate'],
];
const FORMATTERS = {percent, celsius, rate, rateIn, rateOut};

export const Indicator = GObject.registerClass(class Indicator extends PanelMenu.Button {
    _init(settings, openPreferences) {
        super._init(0.5, settings.text('app_title'), false);
        this._settings = settings; this._preferences = openPreferences;
        this.menu._boxPointer.bin.set_child(null);
        this._scroll = new St.ScrollView({hscrollbar_policy: St.PolicyType.NEVER, vscrollbar_policy: St.PolicyType.AUTOMATIC, child: this.menu.box, clip_to_allocation: true});
        this.menu._boxPointer.bin.set_child(this._scroll);
        this.menu.connect('open-state-changed', () => this._constrain());
        this._workareaId = global.display.connect('workareas-changed', () => this._constrain());
        this._fields = new Map(); this._rowItems = [];
        this._cellItems = [];
        this._client = null; this._panel = null; this._sample = null; this._connected = false;
        this._changedId = settings.connect(() => this._rebuild());
        this._appearance = new Gio.Settings({schema_id: 'org.gnome.desktop.interface'});
        this._appearanceId = this._appearance.connect('changed::color-scheme', () => this._style());
        this._contrast = St.Settings.get();
        this._contrastId = this._contrast.connect('notify::high-contrast', () => this._style());
        this._build();
    }
    _build() {
        const settings = this._settings;
        this.set_text_direction(settings.rtl ? Clutter.TextDirection.RTL : Clutter.TextDirection.LTR);
        this.menu.actor.set_text_direction(settings.rtl ? Clutter.TextDirection.RTL : Clutter.TextDirection.LTR);
        this.menu.box.set_text_direction(settings.rtl ? Clutter.TextDirection.RTL : Clutter.TextDirection.LTR);
        this._panel = new St.BoxLayout({yAlign: Clutter.ActorAlign.CENTER});
        this._icon = new St.Icon({gicon: new Gio.FileIcon({file: Gio.File.new_for_path(`${settings.root}/resources/brand-symbolic.svg`)}), style_class: 'system-status-icon'});
        this._panel.add_child(this._icon);
        for (const cell of CELLS) {
            if (!settings.config.display[cell.setting]) continue;
            const box = new St.BoxLayout({yAlign: Clutter.ActorAlign.CENTER});
            box.add_child(new St.Label({text: cell.caption, style_class: 'systeminfo-caption', yAlign: Clutter.ActorAlign.CENTER}));
            for (const field of [...cell.fields, ...(settings.config.display.show_temperatures && cell.temperature ? [cell.temperature] : [])])
                box.add_child(this._value(field, 'systeminfo-value'));
            this._panel.add_child(box);
        }
        this.add_child(this._panel);
        this._header = new PopupMenu.PopupBaseMenuItem({reactive: false});
        this._heading = new St.Label({text: settings.text('app_title'), xExpand: true});
        this._header.add_child(this._heading); this.menu.addMenuItem(this._header);
        this._status = new PopupMenu.PopupMenuItem(settings.text('disconnected'), {reactive: false});
        this.menu.addMenuItem(this._status);
        for (let offset = 0; offset < settings.resource.cells.length; offset += 2) {
            const row = new PopupMenu.PopupBaseMenuItem({reactive: false, can_focus: false});
            for (const spec of settings.resource.cells.slice(offset, offset + 2)) {
                const series = settings.resource.series.find(s => s.key === spec.metric);
                const format = series.unit === 'percent' ? 'percent' : series.unit === 'celsius' ? 'celsius' : 'rate';
                const content = new St.BoxLayout({orientation: Clutter.Orientation.VERTICAL});
                const title = new St.Label({text: settings.text(spec.label)});
                const value = this._value(`${spec.metric}:${format}`, 'systeminfo-value');
                content.add_child(title); content.add_child(value);
                const button = new St.Button({child: content, can_focus: true, reactive: true, x_expand: true,
                    accessible_name: `${settings.text(spec.label)} · ${settings.text('details')}`});
                button.connect('clicked', () => this._openAnalysis(spec.metric));
                button.connect('key-focus-in', () => this._reveal(row));
                row.add_child(button); this._cellItems.push({button, content, title, value});
            }
            this.menu.addMenuItem(row);
        }
        for (const [key, field] of ROWS) {
            const item = new PopupMenu.PopupBaseMenuItem({can_focus: true});
            const title = new St.Label({text: settings.text(key), xExpand: true});
            title.set_text_direction(settings.rtl ? Clutter.TextDirection.RTL : Clutter.TextDirection.LTR);
            title.clutter_text.set_line_alignment(settings.rtl ? Pango.Alignment.RIGHT : Pango.Alignment.LEFT);
            item.add_child(title);
            const value = this._value(field, 'systeminfo-value'); item.add_child(value);
            item.connect('activate', () => this._openAnalysis(field.split(':')[0]));
            this._rowItems.push({item, key, value}); this.menu.addMenuItem(item);
        }
        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
        const copy = new PopupMenu.PopupMenuItem(settings.text('copy_report'));
        copy.connect('activate', () => { if (this._sample) this._copy(JSON.stringify(this._sample)); });
        this._copyItem = copy; this.menu.addMenuItem(copy);
        const retry = new PopupMenu.PopupMenuItem(settings.text('retry'));
        retry.connect('activate', () => this._rebuild()); this.menu.addMenuItem(retry);
        const preferences = new PopupMenu.PopupMenuItem(settings.text('settings'));
        preferences.connect('activate', () => this._preferences()); this.menu.addMenuItem(preferences);
        const website = new PopupMenu.PopupMenuItem(settings.text('website'));
        website.connect('activate', () => Gio.AppInfo.launch_default_for_uri_async(settings.resource.website, global.create_app_launch_context(0, -1), null,
            (_, result) => { try { Gio.AppInfo.launch_default_for_uri_finish(result); } catch { this._status.label.text = settings.text('error'); } }));
        this.menu.addMenuItem(website);
        for (const item of this.menu.box.get_children()) {
            if (item.can_focus) item.connect('key-focus-in', () => this._reveal(item));
        }
        this._style(); this._setConnected(false);
        if (settings.ready) this._client = new Client(settings.config.sampling.interval_seconds, sample => this._apply(sample), connected => this._setConnected(connected), settings.config.transport);
        else this._status.label.text = settings.text(settings.error ? 'error' : 'loading');
    }
    _style() {
        const settings = this._settings;
        const saved = settings.config.display.theme;
        const dark = saved.startsWith('dark') || saved === 'system' && this._appearance.get_string('color-scheme') === 'prefer-dark';
        const contrast = this._contrast.high_contrast || saved.endsWith('-contrast');
        const theme = `${dark ? 'dark' : 'light'}${contrast ? '-contrast' : ''}`;
        const c = settings.resource.themes[theme];
        const m = settings.resource.metrics[settings.config.display.density];
        this._panel.style = `spacing: ${m.gap}px;`;
        for (const child of this._panel.get_children()) {
            if (child instanceof St.BoxLayout) child.style = `spacing: ${m.small}px;`;
        }
        this._header.style = `background-color: ${c.surface}; color: ${c.accent}; border-radius: ${m.radius}px; padding: ${m.row_padding}px ${m.padding}px;`;
        this._heading.style = `font-size: ${settings.resource.typography[settings.locale].label}px; font-weight: bold;`;
        this.menu.box.style = `background-color: ${c.surface}; color: ${c.text}; min-width: ${m.min_width}px;`;
        for (const {button, content, title, value} of this._cellItems) {
            button.style = `background-color: ${c.raised}; color: ${c.text}; border: ${m.border}px solid ${c.border}; border-radius: ${m.radius}px; padding: ${m.row_padding}px; margin: ${m.small}px;`;
            content.style = `spacing: ${m.small}px; min-width: ${m.min_width / 2 - 2 * m.padding}px;`;
            title.style = `font-size: ${settings.resource.typography[settings.locale].label}px; color: ${c.secondary};`;
            value.style = `font-size: ${settings.resource.typography[settings.locale].card}px; font-weight: bold;`;
        }
        for (const {item} of this._rowItems) {
            item.set_text_direction(settings.rtl ? Clutter.TextDirection.RTL : Clutter.TextDirection.LTR);
            item.style = `spacing: ${m.padding}px; padding: ${m.small}px ${m.row_padding}px; min-height: ${m.control}px;`;
        }
        this._constrain();
    }
    _constrain() {
        const monitor = Main.layoutManager.findMonitorForActor(this) ?? Main.layoutManager.primaryMonitor;
        if (!monitor) return;
        const area = global.workspace_manager.get_active_workspace().get_work_area_for_monitor(monitor.index);
        const m = this._settings.resource.metrics[this._settings.config.display.density];
        this._scroll.style = `max-height: ${Math.max(m.control, area.height - m.padding * 2)}px;`;
    }
    _reveal(item) {
        const adjustment = this._scroll.vadjustment;
        const top = item.y;
        const bottom = top + item.height;
        if (top < adjustment.value) adjustment.value = top;
        else if (bottom > adjustment.value + adjustment.page_size) adjustment.value = bottom - adjustment.page_size;
    }
    _value(descriptor, styleClass) {
        const label = new St.Label({text: ABSENT, style_class: styleClass, yAlign: Clutter.ActorAlign.CENTER});
        label.set_text_direction(Clutter.TextDirection.LTR);
        if (!this._fields.has(descriptor)) {
            const [path, formatter] = descriptor.split(':');
            this._fields.set(descriptor, {path: path.split('.'), format: FORMATTERS[formatter], labels: []});
        }
        this._fields.get(descriptor).labels.push(label); return label;
    }
    _apply(sample) {
        this._sample = sample;
        for (const {path, format, labels} of this._fields.values()) {
            let value = sample;
            for (const key of path) value = value?.[key];
            const text = format(value);
            for (const label of labels) if (label.text !== text) label.text = text;
        }
        for (const {item, key, value} of this._rowItems) {
            const name = `${this._settings.text(key)}: \u2068${value.text}\u2069`;
            if (item.accessible_name !== name) item.accessible_name = name;
        }
        const name = `${this._settings.text('app_title')}. ${this._settings.text(sample ? 'connected' : 'disconnected')}. ${this._rowItems.filter(row => ['cpu', 'memory'].includes(row.key)).map(row => row.item.accessible_name).join('. ')}`;
        if (this.accessible_name !== name) this.accessible_name = name;
        if (this._copyItem.reactive !== (sample !== null)) this._copyItem.setSensitive(sample !== null);
    }
    _setConnected(connected) {
        this._connected = connected;
        this._status.label.text = this._settings.text(connected ? 'connected' : 'disconnected');
        if (!connected) this._apply(null);
    }
    _copy(text) { St.Clipboard.get_default().set_text(St.ClipboardType.CLIPBOARD, text); }
    _openAnalysis(metric) {
        this.menu.close();
        try {
            const child = Gio.Subprocess.new(['gjs', '-m', `${this._settings.root}/analysis-window.js`, '--metric', metric], Gio.SubprocessFlags.NONE);
            child.wait_async(null, (source, result) => { try { source.wait_finish(result); } catch { /* Owned launch is reaped. */ } });
        } catch { this._status.label.text = this._settings.text('error'); }
    }
    _teardown() {
        this._client?.stop(); this._client = null;
        this._panel?.destroy(); this._panel = null;
        this.menu.removeAll(); this._fields.clear(); this._rowItems = []; this._cellItems = [];
    }
    _rebuild() { this._teardown(); this._build(); }
    destroy() {
        this._settings.disconnect(this._changedId);
        global.display.disconnect(this._workareaId);
        this._appearance.disconnect(this._appearanceId); this._contrast.disconnect(this._contrastId);
        this._teardown(); this._settings = null; super.destroy();
    }
});
