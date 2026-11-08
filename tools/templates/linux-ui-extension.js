// Loaded only by the isolated acceptance fixture, never shipped in the product.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import Clutter from 'gi://Clutter';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const wait = milliseconds => new Promise(resolve => GLib.timeout_add(GLib.PRIORITY_DEFAULT, milliseconds, () => { resolve(); return GLib.SOURCE_REMOVE; }));
async function until(condition) {
    const deadline = GLib.get_monotonic_time() + 15_000_000;
    while (!condition()) {
        if (GLib.get_monotonic_time() >= deadline) throw new Error('Native UI readiness deadline exceeded');
        await wait(25);
    }
}
function assert(condition, message) { if (!condition) throw new Error(message); }
async function command(argv) {
    const process = Gio.Subprocess.new(argv, Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_PIPE);
    return await new Promise((resolve, reject) => process.communicate_utf8_async(null, null, (source, result) => {
        try { const [, output, errors] = source.communicate_utf8_finish(result); if (!source.get_successful()) throw new Error(errors); resolve(output); }
        catch (error) { reject(error); }
    }));
}
async function set(key, value) { await command(['nddev-systeminfo', 'config', 'set', key, String(value)]); }
async function preferencesReady(settings, window) {
    const title = `${settings.text('app_title')} · ${settings.text('settings')}`;
    const probe = `${GLib.getenv('SYSTEMINFO_TEST_ROOT')}/data/gnome-shell/extensions/systeminfo-ui-test@opennetwork/gtk-inspect.js`;
    const deadline = GLib.get_monotonic_time() + 15_000_000;
    while (true) {
        const result = JSON.parse(await command(['gjs', '-m', probe, title, settings.text('realtime'), String(window.meta_window.get_pid())]));
        if (result.ready) return;
        if (GLib.get_monotonic_time() >= deadline) throw new Error('GTK accessibility did not expose the configured locale and interval');
        await wait(25);
    }
}
async function screenshot(name) {
    const file = Gio.File.new_for_path(`${GLib.getenv('SYSTEMINFO_TEST_ARTIFACTS')}/${name}.png`);
    const stream = file.replace(null, false, Gio.FileCreateFlags.REPLACE_DESTINATION, null);
    try { await new Shell.Screenshot().screenshot(false, stream); }
    finally { stream.close(null); }
}
async function exercise() {
    const uuid = 'rldyour-sysinfo@nddev-opennetwork';
    await until(() => Main.extensionManager.lookup(uuid)?.stateObj?._indicator?._sample && Main.extensionManager.lookup(uuid).stateObj._settings.config.sampling.interval_seconds === 0);
    const extension = Main.extensionManager.lookup(uuid).stateObj;
    let indicator = extension._indicator;
    assert(extension._settings.config.display.show_network === false, 'explicit legacy false was not preserved');
    Main.overview.hide();
    assert(indicator._rowItems.length === 12 && indicator._cellItems.length === 6, 'missing metric cells or readings');
    assert(indicator._sample.memory.used !== null, 'native memory unavailable');
    const captures = [];
    for (const [locale, theme] of [['en', 'light'], ['ru', 'dark'], ['kk', 'light-contrast'], ['ar', 'dark-contrast'], ['zh', 'light']]) {
        await set('display.locale', locale); await set('display.theme', theme);
        await until(() => extension._settings.locale === locale && extension._settings.config.display.theme === theme && extension._indicator._sample);
        indicator = extension._indicator; indicator.menu.open();
        await until(() => indicator.menu.isOpen && indicator.menu.actor.get_width() > 0);
        // Capture synchronizes with the compositor's painted frame, after
        // theme/layout changes have reached the actual popup position.
        await screenshot(`${locale}-${theme}`); captures.push(`${locale}-${theme}`);
        assert(indicator.menu.actor.get_width() <= global.stage.width, 'menu exceeds the display width');
        assert(indicator.menu.actor.get_height() <= global.stage.height, 'menu exceeds the display height');
        const [menuX, menuY] = indicator.menu.actor.get_transformed_position();
        assert(menuX >= 0 && menuY >= 0 && menuX + indicator.menu.actor.get_width() <= global.stage.width && menuY + indicator.menu.actor.get_height() <= global.stage.height, `menu content extends outside the display: ${JSON.stringify({locale, menuX, menuY, width: indicator.menu.actor.get_width(), height: indicator.menu.actor.get_height(), stage_width: global.stage.width, stage_height: global.stage.height})}`);
        assert(indicator.accessible_name.includes(extension._settings.text('app_title')), 'missing accessible name');
        assert(indicator.menu.actor.get_text_direction() === (locale === 'ar' ? Clutter.TextDirection.RTL : Clutter.TextDirection.LTR), 'menu writing direction did not change');
        indicator.menu.close();
    }
    await set('display.locale', 'en');
    await until(() => extension._settings.locale === 'en');
    indicator = extension._indicator;
    indicator.menu.open();
    const last = indicator.menu.box.get_children().at(-1);
    last.grab_key_focus();
    await until(() => global.stage.get_key_focus() === last);
    await screenshot('keyboard-focus'); captures.push('keyboard-focus');
    const [, itemY] = last.get_transformed_position();
    const [, scrollY] = indicator._scroll.get_transformed_position();
    assert(itemY >= scrollY && itemY + last.height <= scrollY + indicator._scroll.height, 'keyboard focus is outside the scroll viewport');
    indicator.menu.close();
    // A real cell opens the separate GTK window. Drive its standard controls
    // through accessibility, then validate the real Rust model on the clipboard.
    indicator._cellItems[0].button.emit('clicked', 1);
    const analysisTitle = extension._settings.text('app_title');
    await until(() => global.get_window_actors().some(actor => actor.meta_window.get_title() === analysisTitle));
    const analysis = global.get_window_actors().find(actor => actor.meta_window.get_title() === analysisTitle);
    const analysisPid = analysis.meta_window.get_pid();
    analysis.meta_window.activate(global.get_current_time());
    const probe = `${GLib.getenv('SYSTEMINFO_TEST_ROOT')}/data/gnome-shell/extensions/systeminfo-ui-test@opennetwork/gtk-inspect.js`;
    async function inspect(value, action) {
        return JSON.parse(await command(['gjs', '-m', probe, analysisTitle, value, String(analysisPid), ...(action ? [action] : [])]));
    }
    async function ready(value) {
        const deadline = GLib.get_monotonic_time() + 15_000_000;
        while (!(await inspect(value)).ready) {
            if (GLib.get_monotonic_time() >= deadline) throw new Error(`Analysis accessibility did not expose ${value}`);
            await wait(25);
        }
    }
    const core = extension._settings.text('logical_core').replace('{id}', '0');
    await ready(core);
    await screenshot('analysis-cpu'); captures.push('analysis-cpu');
    assert((await inspect(core, core)).acted, 'logical core button did not activate');
    await ready(`${core} ·`);
    const pause = extension._settings.text('pause');
    assert((await inspect(pause, pause)).acted, 'pause did not activate');
    await ready(extension._settings.text('paused'));
    const copy = extension._settings.text('copy_chart');
    assert((await inspect(copy, copy)).acted, 'analysis copy did not activate');
    const copiedAnalysis = await new Promise(resolve => St.Clipboard.get_default().get_text(St.ClipboardType.CLIPBOARD, (_, value) => resolve(value)));
    assert(JSON.parse(copiedAnalysis).metric === 'core:0', 'analysis did not use the selected real logical core');
    await screenshot('analysis-core-paused'); captures.push('analysis-core-paused');
    const resume = extension._settings.text('resume');
    assert((await inspect(resume, resume)).acted, 'resume did not activate');
    await ready(extension._settings.text('connected'));
    const close = extension._settings.text('close');
    assert((await inspect(close, close)).acted, 'close did not activate');
    await until(() => !Gio.File.new_for_path(`/proc/${analysisPid}`).query_exists(null));
    indicator._preferences();
    await until(() => global.get_window_actors().some(actor => actor.meta_window.get_title() === 'NDDev Systeminfo · Settings'));
    const window = global.get_window_actors().find(actor => actor.meta_window.get_title() === 'NDDev Systeminfo · Settings');
    window.meta_window.change_workspace(global.workspace_manager.get_active_workspace());
    window.meta_window.activate(global.get_current_time());
    await until(() => window.is_mapped() && window.get_width() > 0 && window.get_height() > 0 && window.meta_window.showing_on_its_workspace());
    await preferencesReady(extension._settings, window);
    await screenshot('preferences-en'); captures.push('preferences-en');
    await set('display.locale', 'ar');
    await until(() => extension._settings.locale === 'ar');
    await command(['gsettings', 'set', 'org.gnome.desktop.a11y.interface', 'high-contrast', 'true']);
    await until(() => extension._indicator._contrast.high_contrast);
    await preferencesReady(extension._settings, window);
    await screenshot('preferences-ar-contrast'); captures.push('preferences-ar-contrast');
    // Exercise actual disable/enable cleanup while the provider is connected.
    await Main.extensionManager._callExtensionDisable(uuid);
    assert(extension._indicator === null && extension._settings === null, 'extension leaked its connection or settings');
    await Main.extensionManager._callExtensionEnable(uuid);
    await until(() => Main.extensionManager.lookup(uuid)?.stateObj?._indicator?._sample);
    indicator = Main.extensionManager.lookup(uuid).stateObj._indicator;
    indicator._copyItem.activate(null);
    const clipboard = await new Promise(resolve => St.Clipboard.get_default().get_text(St.ClipboardType.CLIPBOARD, (_, value) => resolve(value)));
    assert(JSON.parse(clipboard).v === 1, 'native clipboard did not receive the report');
    const root = GLib.getenv('SYSTEMINFO_TEST_ROOT');
    const [, bytes] = Gio.File.new_for_path(`${root}/provider.pid`).load_contents(null);
    const pid = new TextDecoder().decode(bytes).trim();
    Gio.File.new_for_path(`${GLib.getenv('SYSTEMINFO_TEST_ARTIFACTS')}/expected-provider-stop`).replace_contents(new TextEncoder().encode(pid), null, false, Gio.FileCreateFlags.REPLACE_DESTINATION, null);
    await command(['kill', '-TERM', pid]);
    await until(() => indicator._sample === null && !indicator._copyItem.reactive);
    assert(indicator._rowItems.every(row => row.value.text === '—'), 'offline UI retained a stale reading');
    indicator.menu.open(); await screenshot('offline-ar'); captures.push('offline-ar');
    return {passed: true, captures, lifecycle: 'enable/read/analysis/core/pause/copy/resume/close/settings/clipboard/disable/re-enable/offline', high_contrast: true};
}
export default class Acceptance extends Extension {
    enable() {
        void exercise().catch(error => ({passed: false, error: String(error), stack: error.stack})).then(result => {
            const file = Gio.File.new_for_path(`${GLib.getenv('SYSTEMINFO_TEST_ARTIFACTS')}/result.json`);
            file.replace_contents(new TextEncoder().encode(JSON.stringify(result)), null, false, Gio.FileCreateFlags.REPLACE_DESTINATION, null);
        });
    }
    disable() {}
}
