// Contract tests for the source-movie + independent video compositor.
// Full original ActionScript hit-testing still requires live Ruffle testing.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import test from "node:test";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const page = readFileSync(join(root, "src/gfx_live_player.html"), "utf8");
const script = readFileSync(join(root, "src/gfx_live_player.js"), "utf8");

test("video remains behind the Ruffle stage and cannot intercept hover", () => {
    assert.match(page, /#stage\s*\{[^}]*z-index:\s*1\s*;/s);
    assert.match(page, /#background-video\s*\{[^}]*pointer-events:\s*none\s*;[^}]*z-index:\s*0\s*;/s);
    assert.match(page, /ruffle-player\s*\{[^}]*pointer-events:\s*auto\s*;/s);
    assert.match(script, /videoElement\.style\.pointerEvents\s*=\s*"none"/);
    assert.match(script, /player\.style\.pointerEvents\s*=\s*"auto"/);
});

test("main-menu Ruffle playback does not put click-to-unmute UI over Flash buttons", () => {
    assert.match(script, /unmuteOverlay:\s*"hidden"/);
    assert.match(script, /splashScreen:\s*false/);
    assert.match(script, /wmode:\s*\(hasVideo\s*\|\|\s*Number\(report\?\.background_video\?\.submenus\?\.available\)\s*>\s*0\)\s*\?\s*"transparent"\s*:\s*"opaque"/);
    assert.match(script, /letterbox:\s*"on"/);
    assert.match(script, /scale:\s*"showAll"/);
});

test("input health audit counts pointer delivery without injecting fake SWF events", () => {
    assert.match(script, /addEventListener\("pointermove"/);
    assert.match(script, /pointerInput\.overComposition/);
    assert.match(script, /pointerInput\.overRuffle/);
    assert.match(script, /pointerInput\.downRuffle/);
});

test("original submenu video transitions preserve Ruffle hover ownership", () => {
    assert.match(script, /evolve-video:/);
    assert.match(script, /requestOriginalVideo\(line\.slice/);
    assert.match(script, /\.\/submenu-video\/"/);
    assert.match(script, /videoRequests = new Set\(\["modeselect", "options", "founderspopup", "arena", "hunt"\]\)/);
    assert.match(script, /followSourceVideo\(sourceMenuStack\[sourceMenuStack\.length - 1\]\)/);
    assert.match(script, /videoElementSource\.pause\(\)/);
    assert.match(script, /videoElementSource\.hidden = true/);
});
