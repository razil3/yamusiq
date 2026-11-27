// Finder-only window sizing. Match the terminal's TTY, never the front window.
// Screen frames use AppKit's bottom-left coordinates; Terminal uses top-left.
function screenBounds(frame, desktopTop) {
    return {
        x: Math.round(frame.origin.x),
        y: Math.round(desktopTop - frame.origin.y - frame.size.height),
        width: Math.round(frame.size.width),
        height: Math.round(frame.size.height)
    };
}

function overlap(a, b) {
    var width = Math.max(0, Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x));
    var height = Math.max(0, Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y));
    return width * height;
}

function targetBounds(bounds, screens) {
    var chosen = screens[0];
    var largest = -1;
    screens.forEach(function (screen) {
        var area = overlap(bounds, screen.frame);
        if (area > largest) {
            largest = area;
            chosen = screen;
        }
    });
    return chosen.visible;
}

function run(argv) {
    if (argv.length !== 1 || !/^\/dev\/tty[^/]+$/.test(argv[0])) {
        throw new Error("Expected the launch terminal's TTY");
    }
    ObjC.import("AppKit");
    var nativeScreens = $.NSScreen.screens;
    if (!Number(nativeScreens.count)) {
        throw new Error("No display available");
    }
    var primaryFrame = nativeScreens.objectAtIndex(0).frame;
    var desktopTop = primaryFrame.origin.y + primaryFrame.size.height;
    var screens = [];
    for (var i = 0; i < Number(nativeScreens.count); i++) {
        var screen = nativeScreens.objectAtIndex(i);
        screens.push({
            frame: screenBounds(screen.frame, desktopTop),
            visible: screenBounds(screen.visibleFrame, desktopTop)
        });
    }
    var terminal = Application("com.apple.Terminal");
    var windows = terminal.windows();
    for (var w = 0; w < windows.length; w++) {
        var tabs = windows[w].tabs();
        for (var t = 0; t < tabs.length; t++) {
            if (tabs[t].tty() === argv[0]) {
                // Setting bounds does not enter native fullscreen or use UI scripting.
                windows[w].bounds = targetBounds(windows[w].bounds(), screens);
                return;
            }
        }
    }
    throw new Error("Launch terminal window not found");
}
