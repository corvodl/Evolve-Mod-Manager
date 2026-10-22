const {test} = require("node:test");
const assert = require("node:assert/strict");
const {readFileSync} = require("node:fs");
const {join} = require("node:path");
const Runtime = require("../src/gfx_preview_runtime.js");

test("only CurrentMenu NPE and CustomSolo are answered, not game services", () => {
    const host = {};
    const state = Runtime.create();
    state.install(host);
    assert.deepEqual(Object.keys(host).sort(), ["CurrentMenu", "CustomSolo", "NPE"]);
    assert.equal(host.CurrentMenu(), "Main");
    assert.equal(host.NPE(), true);
    assert.equal(host.CustomSolo(), true);
    assert.equal(host.StartMatch, undefined);
    assert.equal(state.get("NativeGameCommand"), undefined);
    assert.equal(state.set("NativeGameCommand", true), false);
    assert.deepEqual(state.snapshot().requests, {CurrentMenu: 1, NPE: 1, CustomSolo: 1});
});

test("source menu can return the selected group to original Flash", () => {
    const state = Runtime.create();
    const host = {};
    state.install(host);
    assert.equal(state.set("CurrentMenu", "Singleplayer"), true);
    assert.equal(host.CurrentMenu(), "Singleplayer");
    assert.equal(state.set("CurrentMenu", "Training"), true);
    assert.equal(host.CurrentMenu(), "Training");
    assert.equal(state.set("CurrentMenu", "../unsafe"), false);
    assert.equal(host.CurrentMenu(), "Training");
});

test("only recognized original CustomCondition flags are evaluated", () => {
    const runtime = Runtime.create();
    for (const [condition, triggered] of [
        ["", false],
        ["CustomCondition=[!NPE]", false],
        ["CustomCondition=[NPE]", true],
        ["CustomCondition=[!CustomSolo]", false],
        ["CustomCondition=[CustomSolo]", true],
    ]) {
        assert.deepEqual(runtime.condition(condition), {known: true, triggered});
    }
    runtime.set("NPE", false);
    runtime.set("CustomSolo", false);
    assert.deepEqual(runtime.condition("CustomCondition=[!NPE]"), {known: true, triggered: true});
    assert.deepEqual(runtime.condition("CustomCondition=[CustomSolo]"), {known: true, triggered: false});
    for (const unverified of [
        "CustomCondition=[!TOSAccepted]",
        "CustomCondition=[AccountOnline]",
        "CustomCondition=[NPE && Online]",
        "Expression=(CustomSolo)"
    ]) {
        assert.deepEqual(runtime.condition(unverified), {known: false, triggered: true});
    }
});

test("only the local preview lifts the eval restriction needed by pinned Ruffle", () => {
    const web = readFileSync(join(__dirname, "../src/gfx_live_web.rs"), "utf8");
    assert.match(web, /script-src 'self' 'wasm-unsafe-eval' 'unsafe-eval'/);
    assert.match(web, /connect-src 'self'/);
    assert.match(web, /frame-ancestors 'none'/);
    assert.match(web, /127\.0\.0\.1/);
});

test("preview page registers host answers before loading the source SWF", () => {
    const page = readFileSync(join(__dirname, "../src/gfx_live_player.html"), "utf8");
    const source = readFileSync(join(__dirname, "../src/gfx_live_player.js"), "utf8");
    assert.ok(page.indexOf('src="./preview-state.js"') < page.indexOf('src="./preview.js"'));
    assert.ok(source.indexOf("previewRuntime.install(window)") < source.indexOf("player.ruffle().load({"));
    assert.match(source, /previewRuntime\.condition\(item\.hidden_when\)/);
    assert.match(source, /previewRuntime\.condition\(item\.disabled_when\)/);
    assert.match(source, /setPreviewMenu\(group\)/);
    assert.doesNotMatch(source, /document\.createElement\(["'](?:canvas|img)["']\).*Training/i);
});
