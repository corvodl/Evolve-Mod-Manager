const {test} = require("node:test");
const assert = require("node:assert/strict");
const {readFileSync} = require("node:fs");
const {join} = require("node:path");
const {runInNewContext} = require("node:vm");
const Runtime = require("../src/gfx_preview_runtime.js");

test("only CurrentMenu NPE and CustomSolo are answered, not game services", () => {
    const host = {};
    const state = Runtime.create();
    state.install(host);
    assert.deepEqual(Object.keys(host).sort(), ["CurrentMenu", "CustomSolo", "NPE"]);
    assert.equal(host.CurrentMenu(), 4); // original MMS_ROOT
    assert.equal(host.NPE(), true);
    assert.equal(host.CustomSolo(), true);
    assert.equal(host.StartMatch, undefined);
    assert.equal(state.get("NativeGameCommand"), undefined);
    assert.equal(state.set("NativeGameCommand", true), false);
    assert.deepEqual(state.snapshot().requests, {CurrentMenu: 1, NPE: 1, CustomSolo: 1});
});

test("original numeric CurrentMenu switches the Flash image-button branch", () => {
    const state = Runtime.create();
    const host = {};
    state.install(host);
    const shouldUseImageButtons = () =>
        [Runtime.MODE_IDS.SOLO, Runtime.MODE_IDS.TUTORIALS, Runtime.MODE_IDS.NPE]
            .includes(Number(host.CurrentMenu()));

    assert.equal(shouldUseImageButtons(), false);
    const groups = [
        ["Singleplayer", 2], // UI_Data Training choices
        ["Training", 2],
        ["Tutorials", 2],
        ["NPE", 3],
        ["Solo", 0],
        ["Extras", 1],
        ["Main", 4]
    ];
    for (const [group, code] of groups) {
        assert.equal(state.set("CurrentMenu", group), true);
        assert.equal(host.CurrentMenu(), code);
        assert.equal(shouldUseImageButtons(), [0, 2, 3].includes(code));
        assert.equal(state.snapshot().currentMenuConstant, [
            "MMS_SOLO", "MMS_EXTRAS", "MMS_TUTORIALS", "MMS_NPE", "MMS_ROOT"
        ][code]);
    }
    assert.equal(state.set("CurrentMenu", "3"), true);
    assert.equal(host.CurrentMenu(), 3);
    assert.equal(state.set("CurrentMenu", -1), false);
    assert.equal(state.set("CurrentMenu", "UnknownNativeMode"), false);
    assert.equal(state.set("CurrentMenu", "../unsafe"), false);
    assert.equal(host.CurrentMenu(), 3);
    assert.ok(Number.isNaN(Number("Training"))); // failed old adapter
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
    assert.match(source, /menuInput\.value = String\(state\.CurrentMenu\)/);
    assert.match(source, /key === "CurrentMenu" \? Number\(control\.value\)/);
    assert.match(page, /value="2">Training \/ Tutorials/);
    assert.match(page, /value="3">NPE/);
    assert.doesNotMatch(source, /document\.createElement\(["'](?:canvas|img)["']\).*Training/i);
});


test("same-build Ruffle font sources accept only bounded staged SWF/GFX imports", () => {
    const script = readFileSync(join(__dirname, "../src/gfx_live_player.js"), "utf8");
    const begin = script.indexOf("const verifiedFontSources = (report) => {");
    const end = script.indexOf("\n    };", begin);
    assert.ok(begin >= 0 && end > begin, "original font source guard exists");
    const callback = runInNewContext(
        script.slice(begin, end + "\n    };".length) + "\nverifiedFontSources;"
    );
    const sources = callback({
        fonts: [
            {movie: "fonts/locfont.gfx", staged: true, define_font_tags: 5},
            {movie: "fonts/glyphs.swf", staged: true, define_font_tags: 1},
            {movie: "fonts/locfont.gfx", staged: true, define_font_tags: 5},
            {movie: "fonts/missing.gfx", staged: false, define_font_tags: 3},
            {movie: "../outside.swf", staged: true, define_font_tags: 1},
            {movie: "https://example.com/bad.swf", staged: true, define_font_tags: 1},
            {movie: "/absolute.swf", staged: true, define_font_tags: 1},
            {movie: "fonts/empty.swf", staged: true, define_font_tags: 0}
        ]
    });
    assert.deepEqual(Array.from(sources), ["./fonts/locfont.gfx", "./fonts/glyphs.swf"]);
    assert.match(script, /fontSources: verifiedFontSources\(report\)/);
    assert.match(script, /fontSources: verifiedFontSources\(screen\)/);
    assert.match(script, /const closedPrompt = dismissOriginalSignin\(\)/);
    assert.doesNotMatch(script, /window\.StartMatch\s*=/);
});
