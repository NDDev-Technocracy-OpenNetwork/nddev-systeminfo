// Read actual GTK accessibility objects on this fixture's private session bus.
import Atspi from 'gi://Atspi';
Atspi.init();
const [title, value, processId, actionName] = ARGV;
let acted = false;
let visited = 0;
function names(node, depth = 0) {
    if (++visited > 5000 || depth > 64) throw new Error('Accessibility tree exceeds the fixture bound');
    const result = [];
    const name = node.get_name(); if (name) result.push(name);
    if (actionName && name === actionName && !acted) {
        const action = node.get_action_iface();
        if (action && action.get_n_actions() > 0) acted = action.do_action(0);
    }
    const count = node.get_child_count();
    for (let index = 0; index < count; index++) {
        const child = node.get_child_at_index(index);
        if (child) result.push(...names(child, depth + 1));
    }
    return result;
}
let ready = false;
try {
    const desktop = Atspi.get_desktop(0);
    for (let index = 0; index < desktop.get_child_count(); index++) {
        const application = desktop.get_child_at_index(index);
        // Shell is the calling compositor; inspect only the separate GTK app.
        if (application.get_process_id() !== Number(processId)) continue;
        for (let windowIndex = 0; windowIndex < application.get_child_count(); windowIndex++) {
            const window = application.get_child_at_index(windowIndex);
            if (window.get_name() !== title) continue;
            ready = names(window).some(name => name.includes(value));
        }
    }
} catch { ready = false; }
print(JSON.stringify({ready, acted}));
