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
    const makeText = (tag, text) => {
        const element = document.createElement(tag);
        element.textContent = String(text);
        return element;
    };
    function loadSourceMenu(report) {
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
            for (const entry of entries.filter(item => item.submenu === group)) {
                const target = String(entry.child_submenu || "");
                const isLocal = groups.has(target) && target !== group;
                const button = makeText("button", entry.label || entry.command || "(unnamed source entry)");
                const explanation = isLocal
                    ? "Browse source submenu: " + target
                    : "Native Evolve command unavailable: " + String(entry.command || "(none)");
                const note = makeText("small", explanation);
                button.appendChild(note);
                button.title = explanation;
                button.disabled = !isLocal;
                if (isLocal) {
                    button.addEventListener("click", () => showGroup(target));
                }
                list.appendChild(button);
            }
            back.hidden = group === "Main";
        };
        if (entries.length) {
            navigation.hidden = false;
            const initial = groups.has("Main") ? "Main" : entries[0].submenu;
            back.addEventListener("click", () => showGroup(initial));
            showGroup(initial);
        } else {
            navigation.hidden = true;
        }
        const stats = report.resources || {};
        const resource = document.getElementById("resource-status");
        const messages = [
            "DDS tiles restored: " + Number(stats.restored_dds_tiles || 0),
            "DDS tiles unresolved/skipped: " + Number(stats.skipped_dds_tiles || 0),
            "Missing imported GFX/SWF movies: " + Number(stats.missing_imported_movies || 0),
            "Same-build Asset Library: " + (stats.has_asset_library ? "configured" : "not configured"),
            "Source build: " + (stats.has_source_build ? "identified" : "unavailable")
        ];
        for (const name of (Array.isArray(stats.unresolved_imports)
            ? stats.unresolved_imports.slice(0, 16) : [])) {
            messages.push("Unresolved movie: " + String(name));
        }
        resource.replaceChildren(...messages.map(item => makeText("li", item)));
        document.getElementById("source-limits").textContent =
            String(report.limitation || "Full CryEngine menu initialization is not available.");
    }
    fetch("./preview-menu.json", { cache: "no-store" })
        .then(res => {
            if (!res.ok) throw new Error("Source menu report is unavailable");
            return res.json();
        })
        .then(loadSourceMenu)
        .catch(e => {
            document.getElementById("source-state").textContent = String(e);
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
                show("Movie requested native FSCommand " + String(command).slice(0, 90)
                     + " (" + String(args).slice(0, 90) + "); blocked in preview.", true);
            });
        }
        if (typeof player.ruffle().setTraceObserver === "function") {
            player.ruffle().setTraceObserver(message => {
                const line = String(message).slice(0, 240);
                if (/error|missing|undefined|failed/i.test(line)) {
                    show("Original ActionScript trace: " + line, true);
                }
            });
        }
        // Only trusted isolated source GFX previews are used here. No service
        // globals are exposed; the movie can access the page only through
        // Ruffle's allowScriptAccess mechanism.
        await player.ruffle().load({
            url: "./movie.swf",
            allowScriptAccess: true,
            autoplay: "on",
            backgroundColor: "#000000"
        });
        show("Ready. GFX Tools buttons apply to this running preview without restarting it.");
        document.getElementById("reset").addEventListener("click", () => {
            player.ruffle().reload().then(() => show("Movie reset in the same window."))
                .catch(e => show("Movie reset failed: " + String(e), true));
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
                    show("Movie reset in this window.");
                } else if (action.kind === "call") {
                    const args = action.arg === null ? [] : [action.arg];
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
