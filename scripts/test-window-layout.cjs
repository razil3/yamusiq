// Run with: node --test scripts/test-window-layout.cjs
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const source = fs.readFileSync(path.join(__dirname, '../assets/macos/maximize-terminal.js'), 'utf8');
function helpers() {
    const context = vm.createContext({});
    vm.runInContext(source, context);
    return context;
}
const plain = value => JSON.parse(JSON.stringify(value));
test('AppKit visible frame excludes menu and Dock in Terminal coordinates', () => {
    const h = helpers();
    assert.deepEqual(plain(h.screenBounds({ origin: { x: 0, y: 77 }, size: { width: 1440, height: 793 } }, 900)),
        { x: 0, y: 30, width: 1440, height: 793 });
});
test('choose the display containing most of the window, including negative origins', () => {
    const h = helpers();
    const screens = [
        { frame: { x: 0, y: 0, width: 1440, height: 900 }, visible: { x: 0, y: 30, width: 1440, height: 793 } },
        { frame: { x: -1920, y: -180, width: 1920, height: 1080 }, visible: { x: -1920, y: -150, width: 1920, height: 1050 } }
    ];
    assert.deepEqual(plain(h.targetBounds({ x: -1500, y: 50, width: 1000, height: 700 }, screens)), screens[1].visible);
    assert.deepEqual(plain(h.targetBounds({ x: 400, y: 50, width: 600, height: 500 }, screens)), screens[0].visible);
});
test('resize only the window whose tab matches the launch TTY', () => {
    const h = helpers();
    const changed = [];
    const window = tty => {
        const win = { tabs: () => [{ tty: () => tty }] };
        Object.defineProperty(win, 'bounds', {
            get: () => () => ({ x: 200, y: 100, width: 600, height: 400 }),
            set: value => changed.push({ tty, bounds: plain(value) })
        });
        return win;
    };
    h.ObjC = { import: () => {} };
    h.$ = { NSScreen: { screens: { count: 1, objectAtIndex: () => ({
        frame: { origin: { x: 0, y: 0 }, size: { width: 1440, height: 900 } },
        visibleFrame: { origin: { x: 0, y: 77 }, size: { width: 1440, height: 793 } }
    }) } } };
    h.Application = () => ({ windows: () => [window('/dev/ttys001'), window('/dev/ttys002')] });
    h.run(['/dev/ttys002']);
    assert.deepEqual(changed, [{ tty: '/dev/ttys002', bounds: { x: 0, y: 30, width: 1440, height: 793 } }]);
    assert.throws(() => h.run(['/dev/ttys999']), /not found/);
    assert.throws(() => h.run(['not a tty']), /TTY/);
});
