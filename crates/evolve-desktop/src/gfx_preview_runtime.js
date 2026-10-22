"use strict";
// Explicit, local-only state for the original Stage 2 Flash menu. These are
// preview assumptions, NOT discovered live game state or native callbacks.
(function (root) {
    const HOST_KEYS = Object.freeze(["CurrentMenu", "NPE", "CustomSolo"]);
    const INITIAL = Object.freeze({
        CurrentMenu: "Main",
        // Show all known authored Training choices in the local preview.
        // The checkboxes can turn these off to compare the original conditions.
        NPE: true,
        CustomSolo: true
    });
    function create(initial = INITIAL) {
        const values = {
            CurrentMenu: typeof initial.CurrentMenu === "string" &&
                /^[A-Za-z0-9_]{1,64}$/.test(initial.CurrentMenu)
                ? initial.CurrentMenu : INITIAL.CurrentMenu,
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
                if (typeof value !== "string" || !/^[A-Za-z0-9_]{1,64}$/.test(value)) return false;
                values.CurrentMenu = value;
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
    const api = Object.freeze({create, HOST_KEYS});
    root.EvolvePreviewRuntime = api;
    if (typeof module !== "undefined" && module.exports) {
        module.exports = api;
    }
})(typeof window !== "undefined" ? window : globalThis);
