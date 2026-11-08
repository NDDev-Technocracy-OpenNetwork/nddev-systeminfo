// Native UI configuration is read and validated by the shared Rust CLI.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

export class Settings {
    constructor(root) {
        this.root = root;
        const [, metadata] = Gio.File.new_for_path(`${root}/metadata.json`).load_contents(null);
        this._legacySchema = JSON.parse(new TextDecoder().decode(metadata))['settings-schema'];
        const [, bytes] = Gio.File.new_for_path(`${root}/resources/ui.json`).load_contents(null);
        this.resource = JSON.parse(new TextDecoder().decode(bytes));
        this.config = JSON.parse(JSON.stringify(this.resource.defaults));
        this._listeners = new Set();
        this._cancellable = new Gio.Cancellable();
        this._processes = new Set();
        this._monitor = null;
        this._generation = 0;
        this._closed = false;
        this.error = null;
        this.ready = false;
        this._resolveLocale();
    }
    _resolveLocale() {
        const preferred = this.config.display.locale === 'system' ? GLib.get_language_names() : [this.config.display.locale];
        this.locale = preferred.map(value => value.split(/[-_.@]/)[0]).find(code => this.resource.catalogs[code]) ?? 'en';
        this.rtl = this.locale === 'ar';
    }
    text(key) {
        const value = this.resource.catalogs[this.locale][key];
        if (value === undefined)
            throw new Error(`Unknown message identifier: ${key}`);
        return value;
    }
    connect(callback) { this._listeners.add(callback); return callback; }
    disconnect(callback) { this._listeners.delete(callback); }
    _changed() { this._resolveLocale(); for (const callback of this._listeners) callback(); }
    async start() {
        try {
            this.path = (await this._run(['config', 'path'])).trim();
            if (this._closed) return;
            if (!Gio.File.new_for_path(this.path).query_exists(null)) {
                const source = Gio.SettingsSchemaSource.new_from_directory(`${this.root}/schemas`, Gio.SettingsSchemaSource.get_default(), false);
                const schema = source.lookup(this._legacySchema, false);
                const legacy = new Gio.Settings({settings_schema: schema});
                const preferences = {};
                for (const key of ['interval', 'show-cpu', 'show-memory', 'show-gpu', 'show-network', 'show-temperatures']) {
                    const value = legacy.get_user_value(key);
                    if (value !== null) preferences[key] = value.deep_unpack();
                }
                await this._run(['config', 'migrate', '--legacy-json', JSON.stringify(preferences)]);
            }
            if (this._closed) return;
            this._watch();
            await this.reload();
        } catch (error) {
            if (!this._closed) { this.error = error.message; this._changed(); }
        }
    }
    _watch() {
        this._monitor?.cancel();
        let directory = Gio.File.new_for_path(this.path).get_parent();
        while (directory && !directory.query_exists(null)) directory = directory.get_parent();
        if (!directory) return;
        this._monitor = directory.monitor_directory(Gio.FileMonitorFlags.WATCH_MOVES, this._cancellable);
        this._monitor.connect('changed', (_, file, other) => {
            const relevant = [file?.get_path(), other?.get_path()].some(path => path && (path === this.path || this.path.startsWith(`${path}/`)));
            if (relevant && !this._closed) { this._watch(); void this.reload(); }
        });
    }
    async reload() {
        const generation = ++this._generation;
        try {
            const document = await this._run(['config', 'show', '--json']);
            if (this._closed || generation !== this._generation) return;
            if (document === this._lastDocument && this.error === null) return;
            this.config = JSON.parse(document); this._lastDocument = document;
            this.error = null; this.ready = true; this._changed();
        } catch (error) {
            if (!this._closed && generation === this._generation) { this.error = error.message; this._changed(); }
        }
    }
    async set(key, value) {
        await this._run(['config', 'set', key, String(value)]);
        await this.reload();
    }
    _run(arguments_) {
        const executable = GLib.find_program_in_path('nddev-systeminfo') ?? GLib.build_filenamev([GLib.get_home_dir(), '.local', 'bin', 'nddev-systeminfo']);
        return new Promise((resolve, reject) => {
            if (this._closed) { reject(new Error('Settings are closed')); return; }
            let process;
            try { process = Gio.Subprocess.new([executable, ...arguments_], Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_PIPE); }
            catch (error) { reject(error); return; }
            this._processes.add(process);
            process.communicate_utf8_async(null, this._cancellable, (source, result) => {
                this._processes.delete(source);
                try {
                    const [, output, errors] = source.communicate_utf8_finish(result);
                    if (!source.get_successful()) throw new Error(errors.trim() || this.text('error'));
                    resolve(output);
                } catch (error) { reject(error); }
            });
        });
    }
    close() {
        this._closed = true;
        this._cancellable.cancel(); this._monitor?.cancel(); this._monitor = null;
        for (const process of this._processes) process.force_exit();
        this._processes.clear(); this._listeners.clear();
    }
}
