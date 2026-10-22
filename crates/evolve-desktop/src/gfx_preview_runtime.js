"use strict";
// Explicit, local-only state for the original Stage 2 Flash menu. These are
// preview assumptions, NOT discovered live game state or native callbacks.
(function (root) {
    const HOST_KEYS = Object.freeze(["CurrentMenu", "NPE", "CustomSolo"]);
    // Verified from the original Stage 2 main_menu.gfx (GFX DefineFunction2):
    // shouldUseImageButtons() uses ToNumber(ExternalInterface.call("CurrentMenu"))
    // and compares it to global MMS_TUTORIALS (2), MMS_NPE (3), MMS_SOLO (0).
    // MMS_EXTRAS=1 and MMS_ROOT=4 are assigned in the same source bytecode.
    // Strings such as "Training" produce NaN and disable image cards.
    const MODE_IDS = Object.freeze({
        SOLO: 0, EXTRAS: 1, TUTORIALS: 2, NPE: 3, ROOT: 4
    });
    const MODE_NAMES = Object.freeze([
        "MMS_SOLO", "MMS_EXTRAS", "MMS_TUTORIALS", "MMS_NPE", "MMS_ROOT"
    ]);
    const GROUP_IDS = Object.freeze({
        main: MODE_IDS.ROOT,
        root: MODE_IDS.ROOT,
        extras: MODE_IDS.EXTRAS,
        singleplayer: MODE_IDS.TUTORIALS,
        training: MODE_IDS.TUTORIALS,
        tutorials: MODE_IDS.TUTORIALS,
        npe: MODE_IDS.NPE,
        solo: MODE_IDS.SOLO,
        customsolo: MODE_IDS.SOLO
    });
    const menuId = value => {
        if (typeof value === "number" && Number.isInteger(value) &&
            value >= MODE_IDS.SOLO && value <= MODE_IDS.ROOT) return value;
        if (typeof value !== "string") return null;
        if (/^[0-4]$/.test(value)) return Number(value);
        const key = value.trim().toLowerCase();
        return Object.prototype.hasOwnProperty.call(GROUP_IDS, key) ? GROUP_IDS[key] : null;
    };
    const INITIAL = Object.freeze({
        CurrentMenu: MODE_IDS.ROOT,
        // Show all known authored Training choices in the local preview.
        // The checkboxes can turn these off to compare the original conditions.
        NPE: true,
        CustomSolo: true
    });
    function create(initial = INITIAL) {
        const values = {
            CurrentMenu: menuId(initial.CurrentMenu) ?? INITIAL.CurrentMenu,
            NPE: initial.NPE === true,
            CustomSolo: initial.CustomSolo === true
        };
        const requests = {CurrentMenu: 0, NPE: 0, CustomSolo: 0};
        const get = name => {
            if (!Object.prototype.hasOwnProperty.call(values, name)) return undefined;
            requests[name]++;
            return values[name];
        };
        const set = (name, value) => {
            if (name === "CurrentMenu") {
                const resolved = menuId(value);
                if (resolved === null) return false;
                values.CurrentMenu = resolved;
                return true;
            }
            if (name !== "NPE" && name !== "CustomSolo") return false;
            if (typeof value !== "boolean") return false;
            values[name] = value;
            return true;
        };
        // UI_Data uses CustomCondition=[Flag] or CustomCondition=[!Flag].
        // Unknown native conditions must stay hidden/disabled, not guessed.
        const condition = raw => {
            if (typeof raw !== "string" || !raw.trim()) {
                return {known: true, triggered: false};
            }
            const match = /^CustomCondition=\[\s*(!?)(NPE|CustomSolo)\s*\]$/i.exec(raw.trim());
            if (!match) return {known: false, triggered: true};
            const key = match[2].toLowerCase() === "npe" ? "NPE" : "CustomSolo";
            const active = values[key];
            return {known: true, triggered: match[1] ? !active : active};
        };
        const snapshot = () => ({
            ...values,
            currentMenuConstant: MODE_NAMES[values.CurrentMenu],
            requests: {...requests}
        });
        // Only these exact query names have no-op/preview equivalents.
        // No CryEngine commands, account state, or game services are bridged.
        const install = target => {
            for (const key of HOST_KEYS) {
                target[key] = () => get(key);
            }
        };
        return {get, set, condition, snapshot, install};
    }
    const api = Object.freeze({create, HOST_KEYS, MODE_IDS, menuId});
    root.EvolvePreviewRuntime = api;
    if (typeof module !== "undefined" && module.exports) {
        module.exports = api;
    }
})(typeof window !== "undefined" ? window : globalThis);
