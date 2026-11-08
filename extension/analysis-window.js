// Lazy, separate GTK process. No GTK or plot preparation runs in GNOME Shell.
import Gtk from 'gi://Gtk?version=4.0';
import Gdk from 'gi://Gdk?version=4.0';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import System from 'system';
import {Settings} from './lib/settings.js';
import {percent, celsius, rate} from './lib/format.js';

const [file] = GLib.filename_from_uri(import.meta.url);
const root = GLib.path_get_dirname(file);
const options = {};
for (let i = 0; i < ARGV.length; i += 2) {
    if (!['--metric', '--config', '--cli'].includes(ARGV[i]) || ARGV[i + 1] === undefined)
        throw new Error('Unsupported analysis option');
    options[ARGV[i].slice(2)] = ARGV[i + 1];
}
const application = new Gtk.Application({application_id: 'org.nddev.opennetwork.systeminfo.analysis', flags: Gio.ApplicationFlags.HANDLES_COMMAND_LINE});
for (const key of ['metric', 'config', 'cli'])
    application.add_main_option(key, 0, GLib.OptionFlags.NONE, GLib.OptionArg.STRING, key, null);
let dashboard = null;

class Dashboard {
    constructor() {
        this.settings = new Settings(root, {executable: options.cli, config: options.config});
        this.resource = this.settings.resource;
        this.metric = options.metric ?? null;
        this.range = this.resource.analysis.history_seconds;
        this.page = 0;
        this.view = null;
        this.live = false;
        this.paused = false;
        this.generation = 0;
        this.process = null;
        this.cancel = null;
        this.retry = 0;
        this.raw = null;
        this.closed = false;
        this.provider = new Gtk.CssProvider();
        this.window = new Gtk.ApplicationWindow({application, title: this.t('app_title'), default_width: 520, default_height: 560});
        this.window.add_css_class('systeminfo');
        Gtk.StyleContext.add_provider_for_display(this.window.get_display(), this.provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION);
        this.gtkSettings = Gtk.Settings.get_for_display(this.window.get_display());
        this.themeId = this.gtkSettings.connect('notify::gtk-theme-name', () => this.style());
        this.appearance = new Gio.Settings({schema_id: 'org.gnome.desktop.interface'});
        this.appearanceId = this.appearance.connect('changed::color-scheme', () => this.style());
        this.contrast = new Gio.Settings({schema_id: 'org.gnome.desktop.a11y.interface'});
        this.contrastId = this.contrast.connect('changed::high-contrast', () => this.style());
        this.changeId = this.settings.connect(() => { this.build(); this.restart(); });
        this.window.connect('close-request', () => { this.close(); return false; });
        this.build();
        void this.settings.start();
    }
    t(key) { return this.settings.text(key); }
    box(horizontal = false) { return new Gtk.Box({orientation: horizontal ? Gtk.Orientation.HORIZONTAL : Gtk.Orientation.VERTICAL, spacing: this.metrics.gap}); }
    label(text, wrap = false) { return new Gtk.Label({label: text, xalign: 0, wrap, selectable: true}); }
    button(key, action) {
        const button = new Gtk.Button({label: this.t(key)});
        button.connect('clicked', action);
        return button;
    }
    cell(caption, action) {
        const button = new Gtk.Button({hexpand: true});
        button.add_css_class('metric-cell');
        const content = this.box();
        const title = this.label(caption); title.add_css_class('secondary');
        const value = this.label('—'); value.add_css_class('metric-value'); value.set_direction(Gtk.TextDirection.LTR);
        content.append(title); content.append(value); button.set_child(content);
        button.update_property([Gtk.AccessibleProperty.LABEL], [caption]);
        button.connect('clicked', action);
        return {button, value, caption};
    }
    present(metric) {
        if (metric) this.metric = metric;
        this.build(); this.window.present(); this.restart();
    }
    build() {
        this.metrics = this.resource.metrics[this.settings.config.display.density];
        const m = this.metrics;
        const rtl = this.settings.rtl;
        this.window.set_direction(rtl ? Gtk.TextDirection.RTL : Gtk.TextDirection.LTR);
        this.window.title = this.t('app_title');
        this.content = this.box();
        for (const side of ['top', 'bottom', 'start', 'end']) this.content[`margin_${side}`] = m.padding;
        this.window.set_child(this.content);
        const header = this.box(true);
        const mark = new Gtk.Image({gicon: new Gio.FileIcon({file: Gio.File.new_for_path(`${root}/resources/brand-symbolic.svg`)}), pixel_size: m.brand});
        header.append(mark);
        const title = this.label(this.t('app_title')); title.hexpand = true; title.add_css_class('heading'); header.append(title);
        header.append(this.button('settings', () => this.preferences()));
        this.content.append(header);
        this.status = this.label(this.t('loading')); this.content.append(this.status);
        this.cells = [];
        const grid = new Gtk.Grid({column_spacing: m.gap, row_spacing: m.gap, column_homogeneous: true});
        this.resource.cells.forEach((spec, index) => {
            const cell = this.cell(this.t(spec.label), () => this.select(spec.metric));
            grid.attach(cell.button, index % 2, Math.floor(index / 2), 1, 1);
            this.cells.push({...cell, spec});
        });
        grid.visible = this.metric === null; this.content.append(grid);
        this.coreCells = []; this.coreKey = null; this.gpuKey = null;
        if (this.metric) {
            const toolbar = this.box(true);
            toolbar.append(this.button('back', () => { this.metric = null; this.build(); this.restart(); }));
            this.pause = this.button(this.paused ? 'resume' : 'pause', () => {
                this.paused = !this.paused; this.pause.label = this.t(this.paused ? 'resume' : 'pause'); this.restart(); this.render();
            });
            toolbar.append(this.pause);
            toolbar.append(this.button('copy_chart', () => { if (this.raw) this.window.get_clipboard().set(this.raw); }));
            this.content.append(toolbar);
            const controls = this.box(true);
            const series = Gtk.DropDown.new_from_strings(this.resource.series.map(s => this.t(s.label)));
            const index = this.resource.series.findIndex(s => s.key === this.metric);
            series.selected = index < 0 ? Gtk.INVALID_LIST_POSITION : index;
            series.hexpand = true; series.tooltip_text = this.t('readings');
            series.connect('notify::selected', () => { if (this.resource.series[series.selected]) this.select(this.resource.series[series.selected].key); });
            controls.append(series);
            const ranges = Gtk.DropDown.new_from_strings(this.resource.analysis.ranges_seconds.map(s => this.t(`range_${s / 60}`)));
            ranges.selected = this.resource.analysis.ranges_seconds.indexOf(this.range);
            ranges.tooltip_text = this.t('range');
            ranges.connect('notify::selected', () => { this.range = this.resource.analysis.ranges_seconds[ranges.selected]; this.restart(); });
            controls.append(ranges); this.content.append(controls);
            const detail = this.box();
            this.scale = this.label(''); detail.append(this.scale);
            this.chart = new Gtk.DrawingArea({content_width: m.min_width, content_height: m.min_width / 2, hexpand: true, vexpand: true});
            this.chart.set_draw_func((_, cr, width, height) => this.draw(cr, width, height));
            detail.append(this.chart);
            const axis = this.box(true);
            axis.append(this.label(`−${this.range / 60} min`));
            const now = new Gtk.Label({label: '0', xalign: 1, hexpand: true}); axis.append(now); detail.append(axis);
            this.statistics = this.label('', true); detail.append(this.statistics);
            this.coverage = this.label('', true); detail.append(this.coverage);
            this.cores = this.box(); detail.append(this.cores);
            this.gpus = this.box(); detail.append(this.gpus);
            const scroll = new Gtk.ScrolledWindow({hexpand: true, vexpand: true, hscrollbar_policy: Gtk.PolicyType.NEVER, child: detail});
            scroll.set_min_content_height(m.min_width); this.content.append(scroll);
        }
        const hint = this.label(this.t('history_hint'), true); hint.add_css_class('secondary'); this.content.append(hint);
        const footer = this.box(true);
        const website = this.button('website', () => {
            Gio.AppInfo.launch_default_for_uri_async(this.resource.website, this.window.get_display().get_app_launch_context(), null,
                (source, result) => { try { Gio.AppInfo.launch_default_for_uri_finish(result); } catch { this.status.label = this.t('error'); } });
        });
        website.tooltip_text = this.t('website_help').replace('{url}', this.resource.website);
        footer.append(website); footer.append(this.button('close', () => this.window.close())); this.content.append(footer);
        this.style(); this.render();
    }
    select(metric) {
        this.metric = metric; this.paused = false;
        this.window.set_default_size(this.metrics.min_width * 3, this.metrics.min_width * 2);
        this.build(); this.restart();
    }
    style() {
        const saved = this.settings.config.display.theme;
        const dark = saved.startsWith('dark') || saved === 'system' && this.appearance.get_string('color-scheme') === 'prefer-dark';
        const contrast = this.contrast.get_boolean('high-contrast') || saved.endsWith('-contrast');
        this.colors = this.resource.themes[`${dark ? 'dark' : 'light'}${contrast ? '-contrast' : ''}`];
        const c = this.colors, m = this.metrics, t = this.resource.typography[this.settings.locale];
        this.provider.load_from_string(`window.systeminfo { background: ${c.surface}; color: ${c.text}; font-size: ${t.body}px; }
            .systeminfo button, .systeminfo dropdown { color: ${c.text}; background: ${c.raised}; border: ${m.border}px solid ${c.border}; border-radius: ${m.radius}px; padding: ${m.small}px ${m.gap}px; }
            .systeminfo button:hover { background: ${c.surface}; }
            .systeminfo button:focus-visible, .systeminfo .selected { outline: ${2 * m.border}px solid ${c.focus}; outline-offset: ${m.border}px; }
            .systeminfo .metric-cell { padding: ${m.row_padding}px; }
            .systeminfo .metric-value { font-size: ${t.card}px; font-weight: 600; font-feature-settings: 'tnum'; }
            .systeminfo .secondary { color: ${c.secondary}; font-size: ${t.label}px; }
            .systeminfo .heading { color: ${c.accent}; font-weight: 600; }`);
        this.chart?.queue_draw();
    }
    stop() {
        ++this.generation;
        if (this.retry) { GLib.source_remove(this.retry); this.retry = 0; }
        this.cancel?.cancel(); this.cancel = null;
        if (this.process) {
            const process = this.process; this.process = null;
            process.force_exit();
            process.wait_async(null, (source, result) => { try { source.wait_finish(result); } catch { /* Reaped owned child. */ } });
        }
    }
    restart() {
        this.stop();
        if (this.paused || this.closed || !this.settings.ready) return;
        this.live = false; this.status.label = this.t('loading');
        const executable = options.cli ?? GLib.find_program_in_path('nddev-systeminfo') ?? `${GLib.get_home_dir()}/.local/bin/nddev-systeminfo`;
        const prefix = options.config ? ['--config', options.config] : [];
        const args = [executable, ...prefix, 'analyze', '--metric', this.metric ?? 'cpu.usage', '--range-seconds', String(this.range), '--columns', String(this.resource.analysis.plot_columns)];
        const generation = this.generation;
        this.cancel = new Gio.Cancellable();
        try {
            this.process = Gio.Subprocess.new(args, Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_SILENCE);
            void this.read(this.process.get_stdout_pipe(), this.cancel, generation).catch(() => this.offline(generation));
        } catch { this.offline(generation); }
    }
    async read(stream, cancel, generation) {
        let parts = [], length = 0;
        while (!this.closed && generation === this.generation) {
            const bytes = await new Promise((resolve, reject) => stream.read_bytes_async(8192, GLib.PRIORITY_DEFAULT, cancel,
                (source, result) => { try { resolve(source.read_bytes_finish(result).get_data()); } catch (error) { reject(error); } }));
            if (!bytes.length) throw new Error('Provider closed');
            let start = 0;
            for (let i = 0; i < bytes.length; i++) {
                if (bytes[i] !== 10) continue;
                const part = bytes.slice(start, i); length += part.length;
                if (length > this.resource.analysis.frame_bytes) throw new Error('Analysis frame exceeds limit');
                parts.push(part);
                const line = new Uint8Array(length); let offset = 0;
                for (const fragment of parts) { line.set(fragment, offset); offset += fragment.length; }
                const raw = new TextDecoder('utf-8', {fatal: true}).decode(line);
                const view = JSON.parse(raw);
                if (generation !== this.generation || this.closed) return;
                this.view = view; this.raw = raw; this.live = true; this.render();
                parts = []; length = 0; start = i + 1;
            }
            if (start < bytes.length) { const part = bytes.slice(start); parts.push(part); length += part.length; }
            if (length > this.resource.analysis.frame_bytes) throw new Error('Analysis frame exceeds limit');
        }
    }
    offline(generation) {
        if (generation !== this.generation || this.closed || this.paused) return;
        this.stop(); this.live = false; this.render();
        this.retry = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, this.settings.config.transport.reconnect_seconds, () => {
            this.retry = 0; this.restart(); return GLib.SOURCE_REMOVE;
        });
    }
    value(value, unit) {
        if (value === null || value === undefined || !Number.isFinite(value)) return '—';
        if (unit === 'percent') return percent(value);
        if (unit === 'celsius') return celsius(value);
        if (unit === 'bytes') return GLib.format_size_full(value, GLib.FormatSizeFlags.IEC_UNITS);
        return rate(value);
    }
    render() {
        this.status.label = this.t(this.paused ? 'paused' : this.live ? 'connected' : 'disconnected');
        for (const cell of this.cells) {
            const spec = this.resource.series.find(s => s.key === cell.spec.metric);
            let value = this.live || this.paused ? this.view?.latest : null;
            for (const key of spec.key.split('.')) value = value?.[key];
            cell.value.label = cell.spec.label === 'history' ? `${Math.floor((this.view?.available_ms ?? 0) / 60000)} min` : this.value(value, spec.unit);
            cell.button.tooltip_text = `${cell.caption}: ${cell.value.label} · ${this.t('details')}`;
        }
        if (!this.metric || !this.view) return;
        const v = this.view, s = v.statistics;
        this.statistics.label = [['current', this.live || this.paused ? s.current : null], ['minimum', s.minimum], ['average', s.average], ['maximum', s.maximum]]
            .map(([key, value]) => `${this.t(key)}: \u2068${this.value(value, v.unit)}\u2069`).join('     ');
        this.coverage.label = this.t('history_coverage').replace('{count}', String(s.samples)).replace('{minutes}', (s.covered_ms / 60000).toFixed(1));
        this.scale.label = `${this.t(v.label).replace('{id}', this.metric.slice(5))} · ${this.value(v.y_max, v.unit)} — ${this.value(v.y_min, v.unit)}`;
        this.chart.update_property([Gtk.AccessibleProperty.LABEL], [`${this.t(v.label).replace('{id}', this.metric.slice(5))}. ${this.statistics.label}`]);
        this.chart.queue_draw(); this.renderCores(); this.renderGPUs();
    }
    clear(box) { for (let child = box.get_first_child(); child; child = box.get_first_child()) box.remove(child); }
    renderCores() {
        this.cores.visible = this.metric === 'cpu.usage' || this.metric.startsWith('core:');
        if (!this.cores.visible) return;
        const cores = this.view.latest?.detail.cores ?? [], size = this.resource.analysis.cores_per_page;
        this.page = Math.min(this.page, Math.max(0, Math.ceil(cores.length / size) - 1));
        const slice = cores.slice(this.page * size, (this.page + 1) * size);
        const key = JSON.stringify(slice.map(c => c.id));
        if (key !== this.coreKey) {
            this.coreKey = key; this.clear(this.cores); this.coreCells = [];
            const navigation = this.box(true);
            navigation.append(this.button('previous', () => { this.page = Math.max(0, this.page - 1); this.coreKey = null; this.renderCores(); }));
            navigation.append(this.label(this.t('core_page').replace('{first}', String(cores.length ? this.page * size + 1 : 0))
                .replace('{last}', String(Math.min(cores.length, (this.page + 1) * size))).replace('{count}', String(cores.length))));
            navigation.append(this.button('next', () => { ++this.page; this.coreKey = null; this.renderCores(); }));
            this.cores.append(navigation);
            const grid = new Gtk.Grid({column_spacing: this.metrics.gap, row_spacing: this.metrics.gap, column_homogeneous: true});
            slice.forEach((core, i) => {
                const cell = this.cell(this.t('logical_core').replace('{id}', String(core.id)), () => this.select(`core:${core.id}`));
                grid.attach(cell.button, i % 4, Math.floor(i / 4), 1, 1); this.coreCells.push({...cell, id: core.id});
            });
            this.cores.append(grid);
        }
        for (const cell of this.coreCells) {
            cell.value.label = percent(this.live || this.paused ? slice.find(c => c.id === cell.id)?.usage : null);
            if (this.metric === `core:${cell.id}`) cell.button.add_css_class('selected'); else cell.button.remove_css_class('selected');
        }
    }
    renderGPUs() {
        this.gpus.visible = this.metric.startsWith('gpu');
        if (!this.gpus.visible) return;
        const detail = this.view.latest?.detail, gpus = detail?.gpus ?? [];
        const key = JSON.stringify([detail?.gpu_enabled, gpus.map(g => [g.id, g.name, g.status])]);
        if (key === this.gpuKey) return;
        this.gpuKey = key; this.clear(this.gpus);
        if (detail?.gpu_enabled === false) {
            this.gpus.append(this.label(this.t('gpu_disabled')));
            this.gpus.append(this.button('enable_gpu', () => { void this.settings.set('sampling.gpu', true).catch(() => { this.status.label = this.t('error'); }); }));
        }
        for (const gpu of gpus) {
            this.gpus.append(this.label(`${gpu.name} · ${this.t(gpu.status === 'available' ? 'connected' : 'unavailable')}`));
            const row = this.box(true);
            for (const [field, label] of [['usage', 'gpu'], ['temperature', 'gpu_temperature'], ['dedicated_used', 'dedicated_memory'], ['shared_used', 'shared_memory']])
                row.append(this.button(label, () => this.select(`gpu:${gpu.id}:${field}`)));
            this.gpus.append(row);
        }
    }
    draw(cr, width, height) {
        const rgb = hex => [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16) / 255);
        const color = hex => cr.setSourceRGB(...rgb(hex));
        color(this.colors.raised); cr.paint(); cr.setLineWidth(this.metrics.border);
        color(this.colors.border);
        for (let i = 0; i <= 4; i++) { cr.moveTo(0, height * i / 4); cr.lineTo(width, height * i / 4); } cr.stroke();
        const v = this.view;
        if (!v || v.y_max <= v.y_min) return;
        const xy = (x, y) => [width * Math.max(0, Math.min(1, 1 - (v.end_ms - x) / (v.range_seconds * 1000))), height * (1 - Math.max(0, Math.min(1, (y - v.y_min) / (v.y_max - v.y_min))))];
        color(this.colors.accent); let last = null;
        for (const p of v.points) {
            const point = xy(p.x_ms, p.value);
            if (last && !p.gap) { cr.moveTo(...last); cr.lineTo(...point); }
            cr.moveTo(...xy(p.x_ms, p.low)); cr.lineTo(...xy(p.x_ms, p.high));
            cr.moveTo(point[0] - this.metrics.border, point[1]); cr.lineTo(point[0] + this.metrics.border, point[1]);
            last = point;
        }
        cr.stroke();
    }
    preferences() {
        const [, metadata] = Gio.File.new_for_path(`${root}/metadata.json`).load_contents(null);
        const uuid = JSON.parse(new TextDecoder().decode(metadata)).uuid;
        Gio.Subprocess.new(['gnome-extensions', 'prefs', uuid], Gio.SubprocessFlags.NONE);
    }
    close() {
        this.closed = true; this.stop(); this.view = null; this.raw = null;
        this.settings.disconnect(this.changeId); this.settings.close();
        this.gtkSettings.disconnect(this.themeId); this.appearance.disconnect(this.appearanceId); this.contrast.disconnect(this.contrastId);
        Gtk.StyleContext.remove_provider_for_display(this.window.get_display(), this.provider);
        dashboard = null;
    }
}
application.connect('command-line', (_, command) => {
    const metric = command.get_options_dict().lookup_value('metric', null)?.deep_unpack() ?? null;
    dashboard ??= new Dashboard(); dashboard.present(metric); return 0;
});
System.exit(application.run([System.programInvocationName, ...ARGV]));
