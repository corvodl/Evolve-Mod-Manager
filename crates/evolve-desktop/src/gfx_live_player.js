"use strict";
// The browser stays open. GFX Tools sends only validated source callbacks via
// loopback GET /next; the movie never gets a native CryEngine service bridge.
(async function () {
    const feedback = document.getElementById("feedback");
    const show = (message, warn = false) => {
        feedback.textContent = message;
        feedback.classList.toggle("warning", warn);
    };
    const sourcePanel = document.getElementById("source-panel");
    const sourceButton = document.getElementById("inspect");
    const toggleSource = (open) => {
        sourcePanel.hidden = !open;
        sourceButton.setAttribute("aria-expanded", String(open));
    };
    sourceButton.addEventListener("click", () => toggleSource(sourcePanel.hidden));
    document.getElementById("source-close").addEventListener("click", () => toggleSource(false));
    const originalButtonEvents = [];
    const renderOriginalEvents = () => {
        document.getElementById("button-events").replaceChildren(
            ...originalButtonEvents.map((event, index) => makeText("li",
                event.kind === "fscommand"
                    ? "#" + (index + 1) + " FSCommand " + event.command +
                      " (" + event.arguments + ") · blocked"
                    : "#" + (index + 1) + " original button " + event.button_id +
                      " · flags " + event.condition_flags + " · native mapping unverified"))
        );
    };
    let previewReport = null;
    let updateSourceGroup = () => {};
    let navigateSourceGroup = () => false;
    let navigateSourceBack = () => false;
    document.getElementById("source-back").addEventListener("click", () => {
        navigateSourceBack();
    });
    const copyEvents = document.getElementById("copy-events");
    copyEvents.addEventListener("click", () => {
        const trace = JSON.stringify({
            format: "evolve-original-button-events-v1",
            events: originalButtonEvents,
            note: "Original SWF button IDs and native FSCommand requests are observed only; native execution is blocked."
        }, null, 2);
        if (navigator.clipboard && navigator.clipboard.writeText) {
            navigator.clipboard.writeText(trace)
                .then(() => show("Copied original Flash event trace."))
                .catch(e => show("Cannot copy trace: " + String(e), true));
        } else {
            show("Clipboard API not available in this browser.", true);
        }
    });
    const makeText = (tag, text) => {
        const element = document.createElement(tag);
        element.textContent = String(text);
        return element;
    };
    function loadSourceMenu(report) {
        previewReport = report;
        const localization = report.localization || {};
        const translations = localization.translations || {};
        const entries = Array.isArray(report.commands) ? report.commands.slice(0, 128) : [];
        const groups = new Set(entries.map(item => item.submenu).filter(Boolean));
        const status = document.getElementById("source-state");
        status.textContent = String(report.menu_status || "No source UI_Data menu was found.");
        const navigation = document.getElementById("source-navigation");
        const list = document.getElementById("source-buttons");
        const groupTitle = document.getElementById("source-section");
        const back = document.getElementById("source-back");
        const showGroup = (group) => {
            list.replaceChildren();
            groupTitle.textContent = group;
            for (const entry of entries.filter(item =>
                item.submenu === group &&
                (!item.platform || String(item.platform).toLowerCase() === "pc") &&
                !item.hidden_when
            )) {
                const target = String(entry.child_submenu || "");
                const isLocal = groups.has(target) && target !== group &&
                    !entry.disabled_when && entries.some(row =>
                        row.submenu === target &&
                        (!row.platform || String(row.platform).toLowerCase() === "pc") &&
                        !row.hidden_when);
                const label = String(entry.label || entry.command || "(unnamed source entry)");
                const translated = label.charAt(0) === "@"
                    ? translations[label.slice(1).toLowerCase()] : undefined;
                const button = makeText("button", translated || label);
                if (translated) button.title = label + " · verified English source translation";
                const explanation = isLocal
                    ? "Browse source submenu: " + target
                    : "Native Evolve command unavailable: " + String(entry.command || "(none)");
                const note = makeText("small", explanation);
                button.appendChild(note);
                button.title = (translated ? label + " · " : "") + explanation;
                button.disabled = !isLocal;
                if (isLocal) {
                    button.dataset.sourceSubmenu = target;
                    button.addEventListener("click", () => {
                        if (navigateSourceGroup(target)) showGroup(target);
                    });
                }
                list.appendChild(button);
            }
            back.hidden = group === "Main";
        };
        updateSourceGroup = showGroup;
        if (entries.length) {
            navigation.hidden = false;
            const initial = groups.has("Main") ? "Main" : entries[0].submenu;
             showGroup(initial);
        } else {
            navigation.hidden = true;
        }
        const statusText = String(localization.status || "No localization manifest available");
        document.getElementById("localization-status").textContent = statusText
            + " (source menu inspector only; does not rewrite Flash text fields).";
        const notFound = Array.isArray(localization.unresolved) ? localization.unresolved : [];
        document.getElementById("localization-unresolved").replaceChildren(
            ...notFound.slice(0, 30).map(item => makeText("li", "Unresolved: @" + String(item)))
        );
        const flashText = report.authored_flash_text || {};
        document.getElementById("localization-status").textContent +=
            " · Original Flash edit text: " + Number(flashText.localized || 0) +
            " localized, " + Number(flashText.unresolved || 0) + " unresolved (" +
            Number(flashText.fields || 0) + " original edit fields).";
        const stats = report.resources || {};
        const resource = document.getElementById("resource-status");
        const messages = [
            "DDS tiles restored: " + Number(stats.restored_dds_tiles || 0),
            "Original SWF button conditions instrumented: " + Number(stats.instrumented_original_button_conditions || 0),
            "DDS tiles unresolved/skipped: " + Number(stats.skipped_dds_tiles || 0),
            "Missing imported GFX/SWF movies: " + Number(stats.missing_imported_movies || 0),
            "Same-build Asset Library: " + (stats.has_asset_library ? "configured" : "not configured"),
            "Source build: " + (stats.has_source_build ? "identified" : "unavailable")
        ];
        for (const font of (Array.isArray(report.fonts) ? report.fonts : [])) {
            messages.push("Font library " + String(font.movie) + ": "
                + (font.staged ? "original source staged" : "original source missing")
                + " · " + (Number.isInteger(font.define_font_tags)
                    ? String(font.define_font_tags) + " original font definitions"
                    : "font definitions not verified")
                + " (" + String(font.source) + ").");
            for (const def of (Array.isArray(font.font_definitions) ? font.font_definitions : []).slice(0, 12)) {
                messages.push("  Original font #" + String(def.font_id) + ": " +
                    String(def.font_name || "(unnamed)") + ", " +
                    String(def.glyphs ?? "unknown") + " glyphs" +
                    (def.bold ? ", bold" : "") + (def.italic ? ", italic" : "") +
                    (def.postscript_name ? ", full name " + String(def.postscript_name) : "") +
                    ". Runtime font binding/selection unverified.");
            }
        }
        const nestedText = report.imported_flash_text || {};
        messages.push("Nested GFX source-authored edit text: " +
            Number(nestedText.localized || 0) + " localized, " +
            Number(nestedText.unresolved || 0) + " unresolved across " +
            Number(nestedText.movies || 0) + " imported movies" +
            " (" + Number(nestedText.unreadable_or_unsupported || 0) + " unsupported). " +
            "Later ActionScript text assignments are not translated.");
        const renderFlags = report.render_effects || {};
        messages.push("Source Flash display-list effects: " +
            Number(renderFlags.authored_color_transforms || 0) + " color transforms, " +
            Number(renderFlags.authored_filter_lists || 0) + " filter lists, " +
            Number(renderFlags.authored_blend_modes || 0) + " blend-mode placements" +
            " (" + Number(renderFlags.nested_sprites || 0) + " nested sprites). " +
            (renderFlags.partial ? "Static effect inventory incomplete." :
                "Ruffle output and runtime Scaleform effects not visually verified."));
        const video = report.background_video || {};
        messages.push("Main-menu motion background: " +
            String(video.status || "Original video source not inspected"));
        if (video.playable) {
            messages.push("Video playback source: " + String(video.mode || "browser video"));
        }
        messages.push("In-movie localized static Flash labels: " +
            Number(report.authored_flash_text?.localized || 0) + " / " +
            Number(report.authored_flash_text?.fields || 0) +
            "; dynamically created native labels not substituted.");
        messages.push("Native CryEngine initialization: unavailable.");
        for (const name of (Array.isArray(stats.unresolved_imports)
            ? stats.unresolved_imports.slice(0, 16) : [])) {
            messages.push("Unresolved movie: " + String(name));
        }
        resource.replaceChildren(...messages.map(item => makeText("li", item)));
        document.getElementById("source-limits").textContent =
            String(report.limitation || "Full CryEngine menu initialization is not available.");
    }
    // Separate CRI USM video from the authored Flash display list. This
    // supports only a verified same-build, locally staged browser derivative.
    // Missing video must remain missing rather than showing guessed artwork.
    function configureMainMenuVideo(report) {
        const video = document.getElementById("background-video");
        const state = report?.background_video || {};
        if (state.playable !== true ||
            !["menu-background.mp4", "menu-background.webm"].includes(state.file)) {
            video.hidden = true;
            return false;
        }
        video.addEventListener("error", () =>
            show("Original main-menu background video could not play in this browser.", true),
            { once: true });
        video.src = "./" + state.file;
        video.hidden = false;
        video.play().catch(error => {
            show("Background video playback needs browser permission: " + String(error), true);
        });
        return true;
    }
    const sourceLoad = fetch("./preview-menu.json", { cache: "no-store" })
        .then(res => {
            if (!res.ok) throw new Error("Source menu report is unavailable");
            return res.json();
        })
        .then(report => { loadSourceMenu(report); return report; })
        .catch(e => {
            document.getElementById("source-state").textContent = String(e);
            return null;
        });
    try {
        if (!window.RufflePlayer || !window.RufflePlayer.newest) {
            throw new Error("Bundled Ruffle Web runtime is unavailable.");
        }
        const runtime = window.RufflePlayer.newest();
        const player = runtime.createPlayer();
        document.getElementById("stage").appendChild(player);
        // Record native FSCommands and AVM trace diagnostics, but deliberately
        // DO NOT execute game commands from an untrusted or modded SWF.
        if (typeof player.ruffle().addFSCommandHandler === "function") {
            player.ruffle().addFSCommandHandler((command, args) => {
                const event = {
                    kind: "fscommand",
                    command: String(command).slice(0, 90),
                    arguments: String(args).slice(0, 90),
                    disposition: "blocked"
                };
                originalButtonEvents.push(event);
                if (originalButtonEvents.length > 64) originalButtonEvents.shift();
                renderOriginalEvents();
                show("Movie requested native FSCommand " + event.command +
                     " (" + event.arguments + "); blocked in preview.", true);
            });
        }
        if (typeof player.ruffle().setTraceObserver === "function") {
            player.ruffle().setTraceObserver(message => {
                const line = String(message).slice(0, 240);
                if (line.startsWith("evolve-button:")) {
                    const parts = line.split(":");
                    const id = Number(parts[1]), flags = Number(parts[2]);
                    if (Number.isInteger(id) && id >= 0 && id <= 65535 &&
                        Number.isInteger(flags) && flags >= 0 && flags <= 65535) {
                        originalButtonEvents.push({
                            kind: "swf_button",
                            button_id: id,
                            condition_flags: flags
                        });
                        if (originalButtonEvents.length > 64) originalButtonEvents.shift();
                        renderOriginalEvents();
                    }
                    show("Original Flash button " + (parts[1] || "?") + " fired condition " + (parts[2] || "?") + ". Native handler mapping remains unverified.");
                } else if (/error|missing|undefined|failed/i.test(line)) {
                    show("Original ActionScript trace: " + line, true);
                }
            });
        }
        // Only trusted isolated source GFX previews are used here. No service
        // globals are exposed; the movie can access the page only through
        // Ruffle's allowScriptAccess mechanism.
        const report = await sourceLoad;
        const hasVideo = configureMainMenuVideo(report);
        // Ruffle's transparent window mode composites the original Flash
        // display list over the separately decoded looping game background.
        // An absent video keeps the default opaque, black diagnostic view.
        await player.ruffle().load({
            url: "./movie.swf",
            allowScriptAccess: true,
            autoplay: "on",
            wmode: hasVideo ? "transparent" : "opaque",
            backgroundColor: hasVideo ? null : "#000000"
        });
        const initButton = document.getElementById("initialize");
        const canOpenScreen = report?.initialization?.verified_open_screen === true;
        const canReturnToMain = report?.initialization?.verified_transition_to_main_menu === true;
        const canInitialize = canOpenScreen || canReturnToMain;
        const originalStartup = canOpenScreen ? "openScreen" : "transitionToMainMenu";
        initButton.disabled = !canInitialize;
        const dismissOverlay = document.getElementById("dismiss-overlay");
        const canDismissPatchNotes = report?.initialization?.verified_hide_patch_notes === true;
        dismissOverlay.disabled = !canDismissPatchNotes;
        dismissOverlay.addEventListener("click", () => {
            if (!canDismissPatchNotes) return;
            try {
                const result = player.ruffle().callExternalInterface("evolvePreview_hidePatchNotes");
                show(result === true
                    ? "Called original hidePatchNotes; check the Flash menu for the visible transition."
                    : "Original hidePatchNotes callback was unavailable at runtime.", result !== true);
            } catch (error) {
                show("Original hidePatchNotes raised: " + String(error), true);
            }
        });
        // Preserve XML order and exact command identifiers. Conditional game
        // state is unavailable: hidden entries stay hidden, conditional actions
        // stay disabled. These are original Flash buttons, not HTML substitutes.
        const translate = key => (report.localization?.translations || {})[key.replace(/^@/, "").toLowerCase()] || key;
        const menuLabels = {play: translate("@ui_mp_OnlineGame"), training: translate("@ui_Training")};
        const sourceRows = Array.isArray(report.commands) ? report.commands : [];
        const groups = new Set(sourceRows.map(item => item.submenu));
        const rowsForGroup = group => sourceRows.filter(item =>
            item.submenu === group && (!item.platform || String(item.platform).toLowerCase() === "pc")
            && !item.hidden_when
        ).slice(0, 128).map(item => ({
            displayName: translate(String(item.label || "")),
            command: item.command, isEnabled: !item.disabled_when,
            isDisabledOnSignIn: Boolean(item.disabled_when), isNew: false,
            isSelectableWhenDisabled: false, setStatListArray: []
        }));
        const menuData = rowsForGroup("Main");
        const canPushSubmenu = report?.initialization?.verified_source_submenus === true;
        const canGoBack = report?.initialization?.verified_go_back === true;
        const sourceMenuStack = ["Main"];
        const pushOriginalSourceMenu = group => {
            if (!canPushSubmenu || !groups.has(group) || !rowsForGroup(group).length ||
                sourceMenuStack.length >= 32 || group === sourceMenuStack[sourceMenuStack.length - 1]) {
                show("Original submenu has no verified, visible source rows.", true);
                return false;
            }
            try {
                const result = player.ruffle().callExternalInterface(
                    "evolvePreview_pushSourceMenu", rowsForGroup(group));
                if (result !== true) throw new Error("Flash callback unavailable");
                sourceMenuStack.push(group);
                show("Original Flash menu state pushed: " + group +
                     ". Native game-service commands remain disabled.");
                return true;
            } catch (error) {
                show("Original submenu transition failed: " + String(error), true);
                return false;
            }
        };
        const goBackOriginalSourceMenu = () => {
            if (!canGoBack || sourceMenuStack.length < 2) return false;
            try {
                const result = player.ruffle().callExternalInterface("evolvePreview_goBack");
                if (result !== true) throw new Error("Original goBack unavailable");
                sourceMenuStack.pop();
                updateSourceGroup(sourceMenuStack[sourceMenuStack.length - 1]);
                show("Original Flash goBack invoked. Visual transition requires Ruffle verification.");
                return true;
            } catch (error) {
                show("Original goBack failed: " + String(error), true);
                return false;
            }
        };
        // The inspector is metadata; transition only after the movie accepts
        // the verified original push/goBack handler. Never route native commands.
        navigateSourceGroup = pushOriginalSourceMenu;
        navigateSourceBack = goBackOriginalSourceMenu;
        const initializeOriginal = async (automatic) => {
            if (!canInitialize) {
                show("No supported source-verified menu lifecycle was found.", true);
                return;
            }
            // The original first-frame callback registration must run before
            // ExternalInterface dispatch. This is an *ActionScript-only* attempt,
            // never a mock game-service or fabricated menu screen.
            for (let attempt = 0; attempt < 5; attempt++) {
                try {
                    const result = player.ruffle().callExternalInterface("evolvePreview_" + originalStartup, menuData, menuLabels);
                    if (result === true) {
                        sourceMenuStack.splice(1);
                        updateSourceGroup("Main");
                        show((automatic ? "Automatic" : "Manual") +
                            " original " + originalStartup + " invoked with source menu rows. Offline preview state; game services remain unavailable.");
                        return;
                    }
                } catch (e) {
                    if (attempt === 4) {
                        show("Original menu initialization raised: " + String(e), true);
                        return;
                    }
                }
                await new Promise(resolve => setTimeout(resolve, 150));
            }
            show("Original menu initialization callback was not ready or registered; no transition confirmed.", true);
        };
        initButton.addEventListener("click", () => { void initializeOriginal(false); });
        show("Ready. Original Flash controls can be traced; native CryEngine host is unavailable.");
        if (canInitialize) {
            await new Promise(resolve => setTimeout(resolve, 200));
            await initializeOriginal(true);
        }
        document.getElementById("reset").addEventListener("click", () => {
            player.ruffle().reload().then(async () => {
                sourceMenuStack.splice(1);
                show("Movie reset in the same window.");
                if (canInitialize) await initializeOriginal(true);
            }).catch(e => show("Movie reset failed: " + String(e), true));
        });
        let alive = true;
        while (alive) {
            try {
                const response = await fetch("./next", { cache: "no-store" });
                if (!response.ok) {
                    throw new Error("Preview host disconnected.");
                }
                const action = await response.json();
                if (action.kind === "reload") {
                    await player.ruffle().reload();
                    sourceMenuStack.splice(1);
                    show("Movie reset in this window.");
                } else if (action.kind === "call") {
                    const args = action.arg === null ? [] : [action.arg];
                    if (action.name === "evolvePreview_openScreen") {
                        await initializeOriginal(false);
                        continue;
                    }
                    let result;
                    try {
                        result = player.ruffle().callExternalInterface(action.name, ...args);
                    } catch (e) {
                        show(action.label + ": ActionScript runtime error: " + String(e), true);
                        await new Promise(resolve => setTimeout(resolve, 250));
                        continue;
                    }
                    // A wrapper returns true only if the allowlisted original
                    // ActionScript handler was actually invoked. Never show a
                    // success tick just because a command was queued.
                    if (result === true) {
                        show("Called original " + action.label
                             + ". Visible changes still depend on source movie state.");
                    } else {
                        show(action.label +
                             ": callback unavailable in Ruffle or the movie was not initialized. No state change confirmed.", true);
                    }
                }
            } catch (e) {
                show("Local GFX preview connection closed: " + String(e), true);
                alive = false;
            }
            if (alive) {
                await new Promise(resolve => setTimeout(resolve, 200));
            }
        }
    } catch (e) {
        show("Cannot initialize live preview: " + String(e), true);
    }
})();
