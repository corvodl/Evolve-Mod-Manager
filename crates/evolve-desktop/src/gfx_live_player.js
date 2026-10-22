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
    let openVerifiedScreen = () => false;
    // Native game commands never execute in the local browser. The source
    // UI_Data manifest can, however, identify a unique UI-only submenu.
    let routeVerifiedLocalCommand = () => false;
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
        const linked = document.getElementById("linked-screen-buttons");
        linked.replaceChildren();
        const screens = Array.isArray(report.linked_screens) ? report.linked_screens : [];
        for (const screen of screens) {
            if (!screen || !/^[a-z0-9-]{1,48}$/.test(String(screen.id || "")) ||
                !/^screens\/[a-z0-9-]{1,48}\/movie\.swf$/.test(String(screen.movie || ""))) {
                continue;
            }
            const button = makeText("button", String(screen.title || screen.id));
            button.appendChild(makeText("small", String(screen.virtual_source || "") +
                " · same-build verified; native routing not established"));
            button.addEventListener("click", () => { void openVerifiedScreen(screen); });
            linked.appendChild(button);
        }
        if (!screens.length) {
            linked.appendChild(makeText("p",
                "No other original GFX screens were verified in this source build. Import the matching UI_Assets PAK."));
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
            "DDS tiles skipped (not necessarily missing): " + Number(stats.skipped_dds_tiles || 0),
            "Missing imported GFX/SWF movies: " + Number(stats.missing_imported_movies || 0),
            "Same-build Asset Library: " + (stats.has_asset_library ? "configured" : "not configured"),
            "Source build: " + (stats.source_build_label ||
                "unavailable — open an exact-build library movie or explicitly select a verified source build")
        ];
        const imageIssues = stats.bitmap_skip_details || {};
        messages.push("Original DDS skips by cause: " +
            Number(imageIssues.missing_source || 0) + " missing source tiles, " +
            Number(imageIssues.preview_limit || 0) + " bounded preview limits, " +
            Number(imageIssues.duplicate_or_embedded || 0) + " reused or already defined, " +
            Number(imageIssues.invalid_tile || 0) + " invalid atlas tiles, " +
            Number(imageIssues.decode_error || 0) + " DDS decoding failures.");
        messages.push("Preview limit breakdown: " +
            Number(imageIssues.bitmap_count_limit || 0) + " bitmap-count limit, " +
            Number(imageIssues.pixel_limit || 0) + " tile/pixel budget, " +
            Number(imageIssues.movie_size_limit || 0) + " reconstructed-SWF byte limit.");
        for (const file of (Array.isArray(imageIssues.missing_paths)
            ? imageIssues.missing_paths.slice(0, 12) : [])) {
            messages.push("Missing original DDS: " + String(file));
        }
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
        const modeImages = report.mode_images || {};
        if (Number(modeImages.requested || 0) > 0) {
            messages.push("Original MenuModeSelect images: " +
                Number(modeImages.staged || 0) + " / " +
                Number(modeImages.requested || 0) +
                " source-qualified DDS staged as temporary PNGs. " +
                "Other CryEngine img:// image families remain unavailable.");
            for (const key of ["missing", "ambiguous_or_invalid_manifest", "decode_or_budget_failures"]) {
                const rows = Array.isArray(modeImages[key]) ? modeImages[key] : [];
                if (rows.length) messages.push("Mode DDS " + key + ": " + rows.slice(0, 12).join(", "));
            }
        }
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
        const stageElement = document.getElementById("stage");
        stageElement.appendChild(player);
        const linkedBack = document.getElementById("linked-screen-back");
        const videoElement = document.getElementById("background-video");
        let linkedPlayer = null;
        const returnToOriginalMenu = () => {
            if (!linkedPlayer) return false;
            linkedPlayer.remove();
            linkedPlayer = null;
            player.hidden = false;
            videoElement.hidden = !videoElement.src;
            linkedBack.hidden = true;
            show("Returned to the original main-menu instance; its ActionScript state was not reset.");
            return true;
        };
        linkedBack.addEventListener("click", returnToOriginalMenu);
        openVerifiedScreen = async screen => {
            if (!screen || !/^screens\/[a-z0-9-]{1,48}\/movie\.swf$/.test(String(screen.movie || ""))) {
                show("Linked screen rejected: not a staged, verified source movie.", true);
                return false;
            }
            returnToOriginalMenu();
            const candidate = runtime.createPlayer();
            stageElement.appendChild(candidate);
            try {
                if (typeof candidate.ruffle().addFSCommandHandler === "function") {
                    candidate.ruffle().addFSCommandHandler((command, args) => {
                        show("Linked GFX requested native FSCommand " + String(command).slice(0, 90) +
                            " (" + String(args).slice(0, 90) + "); blocked in preview.", true);
                    });
                }
                await candidate.ruffle().load({
                    url: "./" + screen.movie,
                    allowScriptAccess: false,
                    autoplay: "on",
                    wmode: "opaque",
                    backgroundColor: "#000000"
                });
                linkedPlayer = candidate;
                player.hidden = true;
                videoElement.hidden = true;
                linkedBack.hidden = false;
                show("Previewing verified " + String(screen.title) +
                     ". Original native CryEngine screen lifecycle is unavailable.");
                return true;
            } catch (error) {
                candidate.remove();
                show("Cannot open source movie: " + String(error), true);
                return false;
            }
        };
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
                // Only original UI_Data child_submenu entries are eligible,
                // never a native match/tutorial, game service, or FSCommand
                // supplied by an unknown movie.
                if (routeVerifiedLocalCommand(event.command)) {
                    event.disposition = "local-only";
                }
                originalButtonEvents.push(event);
                if (originalButtonEvents.length > 64) originalButtonEvents.shift();
                renderOriginalEvents();
                if (event.disposition === "blocked") {
                    show("Movie requested native FSCommand " + event.command +
                         " (" + event.arguments + "); blocked in preview.", true);
                }
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
        const auditButton = document.getElementById("audit-assets");
        auditButton.disabled = !report;
        auditButton.addEventListener("click", async () => {
            if (!report || auditButton.disabled) return;
            auditButton.disabled = true;
            try {
                const staged = report.mode_images?.staged_names;
                const names = Array.isArray(staged)
                    ? staged.filter(name => typeof name === "string" && /^[A-Za-z0-9_-]{1,64}$/.test(name)).slice(0, 64)
                    : [];
                let decoded = 0;
                const failures = [];
                // The report's staged count alone does not prove that the
                // browser can fetch/decode those same-build DDS derivatives.
                await Promise.all(names.map(async name => {
                    try {
                        const response = await fetch("./runtime-images/" + name + ".png", {cache: "no-store"});
                        if (!response.ok || !String(response.headers.get("content-type")).includes("image/png")) {
                            throw new Error("HTTP " + response.status);
                        }
                        const bytes = await response.arrayBuffer();
                        const view = new DataView(bytes);
                        if (bytes.byteLength < 24 || view.getUint32(0, false) !== 0x89504e47 ||
                            view.getUint32(4, false) !== 0x0d0a1a0a) {
                            throw new Error("invalid PNG signature");
                        }
                        const width = view.getUint32(16, false);
                        const height = view.getUint32(20, false);
                        if (width === 0 || height === 0 || width > 8192 || height > 8192) {
                            throw new Error("invalid PNG dimensions");
                        }
                        decoded++;
                    } catch (error) {
                        failures.push(name + ": " + String(error).slice(0, 80));
                    }
                }));
                const meta = player.ruffle().metadata || {};
                const stageDimensions = String(meta.width || "?") + "x" + String(meta.height || "?");
                const video = document.getElementById("background-video");
                const state = report.background_video || {};
                const playback = state.playable === true
                    ? "readyState=" + video.readyState + ", size=" +
                      (video.videoWidth || 0) + "x" + (video.videoHeight || 0) +
                      ", paused=" + String(video.paused) + ", error=" +
                      (video.error?.code || "none")
                    : String(state.status || "source unavailable");
                const fonts = (Array.isArray(report.fonts) ? report.fonts : []);
                const knownFonts = fonts.filter(item => Number(item.define_font_tags || 0) > 0).length;
                const text = "Ruffle source stage " + stageDimensions + "; browser PNG images " +
                    decoded + "/" + names.length + "; embedded Flash bitmap linkages " +
                    Number(report.mode_images?.flash_bitmap_linkages || 0) +
                    "; recognized font libraries " + knownFonts + "/" + fonts.length +
                    "; Goliath video " + playback +
                    (failures.length ? "; asset fetch/decode errors: " + failures.slice(0, 5).join("; ") : "");
                const entry = makeText("li", text);
                document.getElementById("resource-status").prepend(entry);
                show(text, failures.length > 0 || (names.length > 0 && decoded !== names.length));
            } finally {
                auditButton.disabled = false;
            }
        });
        const notice = document.getElementById("preview-notice");
        const reasons = [];
        if (!hasVideo && report?.background_video?.source?.source_archive === "videos.pak") {
            reasons.push("Original moving menu background not displayed: " +
                String(report.background_video.status ||
                       "the original videos.pak source could not be prepared."));
        }
        const unresolved = report?.localization?.unresolved;
        if (!report?.resources?.has_source_build) {
            reasons.push("No verified source build: original UI_Data, English text, font imports and video cannot be resolved across the library until the build is identified.");
        }
        if (Array.isArray(unresolved) && unresolved.includes("ui_signinginprompt")) {
            reasons.push("@ui_SigningInPrompt is untranslated. Verify same-build English UI strings; dynamic ActionScript text may also depend on the native game host.");
        }
        notice.textContent = reasons.join("\n");
        notice.hidden = reasons.length === 0;
        // Ruffle's transparent window mode composites the original Flash
        // display list over the separately decoded looping game background.
        // An absent video keeps the default opaque, black diagnostic view.
        await player.ruffle().load({
            url: "./movie.swf",
            allowScriptAccess: true,
            autoplay: "on",
            wmode: hasVideo ? "transparent" : "opaque",
            backgroundColor: hasVideo ? null : "#000000",
            // CryEngine implements img:// for dynamic menu card artwork.
            // Ruffle supports URL rewriting; only the specific original
            // MenuModeSelect DDS family is mapped to source-verified PNG copies
            // served from this preview's isolated, local-only session.
            urlRewriteRules: [
                [/^img:\/\/Libs\/UI\/FlashAssets\/images\/MenuModeSelect\/([A-Za-z0-9_-]+)\.dds$/i,
                 "./runtime-images/$1.png"]
            ]
        });
        // Ruffle uses the source SWF stage dimensions (not the containing
        // browser window). Keep the independently decoded original video and
        // the transparent Flash display list in the *same* fitted rectangle,
        // avoiding differing crop/letterbox behavior on non-16:9 windows.
        const composition = document.getElementById("composition");
        const hostStage = document.querySelector("main");
        const fitSourceStage = () => {
            const source = player.ruffle().metadata;
            const width = Number(source?.width), height = Number(source?.height);
            const bounds = hostStage.getBoundingClientRect();
            if (!Number.isFinite(width) || !Number.isFinite(height) ||
                width <= 0 || height <= 0 || bounds.width <= 0 || bounds.height <= 0) {
                return;
            }
            const factor = Math.min(bounds.width / width, bounds.height / height);
            composition.style.width = Math.round(width * factor) + "px";
            composition.style.height = Math.round(height * factor) + "px";
        };
        fitSourceStage();
        if (typeof ResizeObserver === "function") {
            new ResizeObserver(fitSourceStage).observe(hostStage);
        } else {
            window.addEventListener("resize", fitSourceStage);
        }
        const initButton = document.getElementById("initialize");
        const canOpenScreen = report?.initialization?.verified_open_screen === true;
        const canReturnToMain = report?.initialization?.verified_transition_to_main_menu === true;
        const canInitialize = canOpenScreen || canReturnToMain;
        const originalStartup = canOpenScreen ? "openScreen" : "transitionToMainMenu";
        initButton.disabled = !canInitialize;
        const dismissOverlay = document.getElementById("dismiss-overlay");
        const canDismissPatchNotes = report?.initialization?.verified_hide_patch_notes === true;
        dismissOverlay.disabled = !canDismissPatchNotes;
        const dismissSignin = document.getElementById("dismiss-signin");
        const canDismissSignin = report?.initialization?.verified_hide_signing_in === true;
        dismissSignin.disabled = !canDismissSignin;
        dismissSignin.addEventListener("click", () => {
            if (!canDismissSignin) return;
            try {
                // Invoke only the source-verified UI-local ActionScript function.
                // Never simulate game authentication or register native services.
                const result = player.ruffle().callExternalInterface(
                    "evolvePreview_showSigningInPrompt", false);
                show(result === true
                    ? "Called original showSigningInPrompt(false); inspect Flash for the transition."
                    : "Original sign-in dismissal was unavailable at runtime.", result !== true);
            } catch (error) {
                show("Original sign-in dismissal raised: " + String(error), true);
            }
        });
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
        // Local routing is strictly tied to one visible, enabled, exact source
        // UI_Data command in the CURRENT original submenu. This never turns a
        // source FSCommand into a native game-service invocation.
        routeVerifiedLocalCommand = command => {
            const current = sourceMenuStack[sourceMenuStack.length - 1];
            const eligible = sourceRows.filter(item =>
                item.submenu === current &&
                item.command === command &&
                typeof item.child_submenu === "string" &&
                groups.has(item.child_submenu) &&
                item.child_submenu !== current &&
                !item.disabled_when && !item.hidden_when &&
                (!item.platform || String(item.platform).toLowerCase() === "pc")
            );
            if (eligible.length !== 1) return false;
            if (!pushOriginalSourceMenu(eligible[0].child_submenu)) return false;
            updateSourceGroup(eligible[0].child_submenu);
            return true;
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
            returnToOriginalMenu();
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
                    returnToOriginalMenu();
                    await player.ruffle().reload();
                    sourceMenuStack.splice(1);
                    show("Movie reset in this window.");
                } else if (action.kind === "call") {
                    returnToOriginalMenu();
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
