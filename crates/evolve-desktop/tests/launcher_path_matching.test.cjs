"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const source = fs.readFileSync(
  path.join(__dirname, "../launcher_support/auto_launch_frida.py"), "utf8"
);
const begin = source.indexOf("function canon(raw) {");
const end = source.indexOf("function readWide(p) {", begin);
assert.ok(begin > 0 && end > begin, "Extract the actual Frida matcher from the shipped helper");
const matcherCode = source.slice(begin, end);

const DRIVE = "C:\\Games\\ModdedEvolve\\EvolveGame\\bin64_SteamRetail\\Evolve.exe";
const EXTENDED_DRIVE = "\\\\?\\" + DRIVE;
const UNC = "\\\\server\\share\\EvolveGame\\bin64_SteamRetail\\Evolve.exe";
const EXTENDED_UNC = "\\\\?\\UNC\\" + UNC.slice(2);

function matcher(expected) {
  return vm.runInNewContext(
    "(() => { const path = " + JSON.stringify(expected) + "; " + matcherCode + " return { matches, canon }; })()"
  );
}

test("Test B: standard CreateProcessW path matches the extended journal game path", () => {
  assert.equal(matcher(EXTENDED_DRIVE).matches(DRIVE), true);
  assert.equal(matcher(DRIVE).matches(EXTENDED_DRIVE), true);
  assert.equal(matcher(EXTENDED_DRIVE).matches('"' + DRIVE + '" -log'), true);
  assert.equal(matcher(EXTENDED_DRIVE).matches('"' + EXTENDED_DRIVE + '" /offline'), true);
});

test("case, slash spelling and quoted command boundaries", () => {
  const m = matcher(EXTENDED_DRIVE);
  assert.equal(m.matches(DRIVE.toUpperCase().replaceAll("\\", "/")), true);
  assert.equal(m.matches('  "' + DRIVE + '"  -log  '), true);
  assert.equal(m.matches(DRIVE + "evil"), false);
  assert.equal(m.matches(DRIVE + "\\child.exe"), false);
  assert.equal(m.matches('"' + DRIVE + '"evil'), false);
  assert.equal(m.matches('"' + DRIVE + 'evil"'), false);
  assert.equal(m.matches('"' + DRIVE), false);
  assert.equal(m.matches(DRIVE.replace("Evolve.exe", "Other.exe")), false);
});

test("extended UNC retains its network root while normalizing names", () => {
  const m = matcher(EXTENDED_UNC);
  assert.equal(m.canon(EXTENDED_UNC), m.canon(UNC));
  assert.ok(m.canon(EXTENDED_UNC).startsWith("\\\\server\\share\\"));
  assert.equal(m.matches(UNC), true);
  assert.equal(matcher(UNC).matches(EXTENDED_UNC), true);
  assert.equal(m.matches('"' + UNC + '" -log'), true);
  assert.equal(m.matches(UNC.replace("Evolve.exe", "Other.exe")), false);
});
