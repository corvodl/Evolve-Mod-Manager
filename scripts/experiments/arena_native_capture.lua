-- Stage 2 native Arena code capture. READ-ONLY.
-- User-owned Evolve.exe 3.6.1.2538 mapped/decrypted image.
-- Dump small, build-specific control-flow regions for offline investigation.
-- Game dump files are NOT included in this repository or public PR.
--
-- Run in Cheat Engine Lua Engine while Evolve.exe is at MAIN MENU.
-- The output files go into %TEMP%; archive them locally for analysis.
-- This does not patch any instruction or spawn an actor.

local SLOTS_RVA=0x15CF820
local EXPECTED_STOCK={0xC7,0x41,0x08,0x04,0x00,0x00,0x00,
                      0xC7,0x41,0x0C,0x01,0x00,0x00,0x00}
local EXPECTED_ONE_MERC={0xC7,0x41,0x08,0x01,0x00,0x00,0x00,
                         0xC7,0x41,0x0C,0x01,0x00,0x00,0x00}
-- Bounds verified from the read-only RVA analysis in PR #135.
-- These ranges are diagnostic samples, NOT runnable native engine call ABIs.
local REGIONS={
  {"slot_factory",     0x15CF400,0x700},
  {"team_capacity",    0x1124100,0x400},
  {"set_team",         0x112DB00,0x700},
  {"entity_factory",   0x009D6500,0x1500},
  {"monster_init",     0x1A72500,0x700},
  {"team_name",        0x00CDB000,0x500},
  {"slot_flags",       0x0015D600,0x500},
  {"team_join_calls",  0x1842000,0xC00},
  {"team_join_alt",    0x17A0000,0x600},
}
local function equal(a,b)
  if not a or #a~=#b then return false end
  for i=1,#b do if a[i]~=b[i] then return false end end
  return true
end
local function read(at,n)
  local ok,bytes=pcall(readBytes,at,n,true)
  if ok and type(bytes)=="table" and #bytes==n then return bytes end
end
local function blob(bytes)
  local parts={}
  for _,byte in ipairs(bytes) do parts[#parts+1]=string.char(byte) end
  return table.concat(parts)
end
local function capture()
  local pid=getProcessIDFromProcessName("Evolve.exe")
  assert(pid and pid~=0,"Evolve.exe is not running")
  if getOpenedProcessID()~=pid then openProcess(pid) end
  assert(getOpenedProcessID()==pid,"CE did not attach")
  local base=getAddressSafe("Evolve.exe")
  assert(type(base)=="number" and base>=0x10000,"Missing module base")
  assert(equal(read(base,2),{0x4D,0x5A}),"Not an MZ module")
  local original=read(base+SLOTS_RVA,#EXPECTED_STOCK)
  assert(equal(original,EXPECTED_STOCK) or equal(original,EXPECTED_ONE_MERC),
    "Build mismatch or other slot patch: capture aborted")
  local tmp=os.getenv("TEMP")
  assert(type(tmp)=="string" and #tmp>2,"TEMP not defined")
  local prefix=tmp.."\\EvolveArenaNative_"
  local manifest={
    "STAGE 2 NATIVE ARENA CAPTURE (READ-ONLY)",
    "Game version assumed: Evolve.exe 3.6.1.2538 (cross-checked by initializer bytes)",
    "PID: "..pid,
    string.format("Module base: 0x%X",base),
    "Slot initializer: "..(equal(original,EXPECTED_STOCK) and "stock 4 merc / 1 monster" or "experiment 1 merc / 1 monster"),
    "Regions are addressed as RVA; image uses ASLR.",
    "The dumped bytes do NOT prove an engine function signature or safe entry point.",
    "Do not commit binaries, game assets or your own private data to the public repo.",
  }
  for _,entry in ipairs(REGIONS) do
    local name,rva,len=entry[1],entry[2],entry[3]
    local bytes=read(base+rva,len)
    assert(bytes,"Cannot read region "..name.." at RVA "..string.format("%X",rva))
    local path=prefix..name..".bin"
    local f=assert(io.open(path,"wb"))
    assert(f:write(blob(bytes)))
    f:close()
    manifest[#manifest+1]=string.format("%s RVA=0x%X SIZE=0x%X FILE=%s",
      name,rva,len,path)
  end
  local f=assert(io.open(prefix.."manifest.txt","wb"))
  assert(f:write(table.concat(manifest,"\r\n").."\r\n"))
  f:close()
  for _,line in ipairs(manifest) do print(line) end
  print("Capture complete. Only your local TEMP directory was written.")
end

local ok,err=pcall(capture)
if not ok then print("[Arena native capture] FAILED: "..tostring(err)) end
