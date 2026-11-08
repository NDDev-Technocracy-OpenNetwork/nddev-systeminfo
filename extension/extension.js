/* rldyour-sysinfo — GNOME Shell panel indicator
 */

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {Indicator} from './lib/indicator.js';
import {Settings} from './lib/settings.js';

/** First slot of the centre box, which is the position left of the clock. */
const PANEL_BOX = 'center';
const PANEL_POSITION = 0;

export default class SysinfoExtension extends Extension {
    enable() {
        this._settings = new Settings(this.path);
        this._indicator = new Indicator(this._settings, () => this.openPreferences());
        Main.panel.addToStatusArea(this.uuid, this._indicator, PANEL_POSITION, PANEL_BOX);
        void this._settings.start();
    }

    disable() {
        // The shell may call disable without a completed enable — when a
        // lock screen interrupts startup, for instance.
        this._indicator?.destroy();
        this._indicator = null;
        this._settings?.close();
        this._settings = null;
    }
}
