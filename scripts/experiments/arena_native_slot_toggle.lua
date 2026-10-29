-- Stage 2 Arena native slot-count isolation experiment (Cheat Engine Lua).
-- Limited to Evolve.exe 3.6.1.2538 live signature. EXPLICIT MEMORY WRITE.
-- Changes desired MERC slots 4 -> 1 while keeping desired MONSTER slots at 1.
-- This DOES NOT spawn a second monster, change bot behavior, or allow 2 users.
-- Never run during an active match. Never use in public/matchmade play.
--
-- From Cheat Engine Lua Engine:
--    arenaNativeSlotStatus()
--    arenaNativeOneMercSlot()      -- at MAIN MENU, before starting local Arena
--    arenaNativeRestoreSlots()    -- after returning to MAIN MENU
-- Full process exit also discards this memory-only change.

local SLOT_RVA = 0x15CF820
local ORIGINAL = {0xC7,0x41,0x08,0x04,0x00,0x00,0x00,
                  0xC7,0x41,0x0C,0x01,0x00,0x00,0x00}
local PATCHED = {0xC7,0x41,0x08,0x01,0x00,0x00,0x00,
                 0xC7,0x41,0x0C,0x01,0x00,0x00,0x00}
local state = {pid=nil,base=nil,active=false}

local function same(a,b)
  if not a or #a ~= #b then return false end
  for i=1,#b do if a[i] ~= b[i] then return false end end
  return true
end
local function dump(bytes)
  if not bytes then return "unreadable" end
  local out={}
  for _,v in ipairs(bytes) do out[#out+1]=string.format("%02X",v) end
  return table.concat(out," ")
end
local function getSession()
  local pid=getProcessIDFromProcessName("Evolve.exe")
  assert(pid and pid ~= 0,"Evolve.exe is not running")
  if getOpenedProcessID() ~= pid then openProcess(pid) end
  assert(getOpenedProcessID()==pid,"Cannot attach to Evolve.exe")
  local base=getAddressSafe("Evolve.exe")
  assert(type(base)=="number" and base>=0x10000,"Evolve.exe base unavailable")
  local mz=readBytes(base,2,true)
  assert(same(mz,{0x4D,0x5A}),"Unsupported module header")
  local target=base+SLOT_RVA
  local observed=readBytes(target,#ORIGINAL,true)
  assert(type(observed)=="table" and #observed==#ORIGINAL,
    "Slot initializer bytes unreadable")
  return pid,base,target,observed
end
local function withError(label,action)
  local ok,errorText=pcall(action)
  if not ok then
    print("[Arena native slots] "..label.." REJECTED: "..tostring(errorText))
  end
end
function arenaNativeSlotStatus()
  withError("status",function()
    local pid,base,_,bytes=getSession()
    print(string.format("[Arena native slots] PID=%d base=0x%X",pid,base))
    if same(bytes,ORIGINAL) then
      print("stock defaults: desired merc=4, monster=1")
    elseif same(bytes,PATCHED) then
      print("experiment: desired merc=1, monster=1")
    else
      print("UNKNOWN CODE BYTES (no changes permitted): "..dump(bytes))
    end
    print("A desired-slot count is NOT the number of occupied players.")
  end)
end
function arenaNativeOneMercSlot()
  withError("apply",function()
    assert(not state.active,"Patch already applied by this script")
    local pid,base,target,observed=getSession()
    assert(same(observed,ORIGINAL),
      "Expected exact stock bytes; refusing an unknown build or another active patch: "..dump(observed))
    assert(type(writeBytes)=="function","CE writeBytes is unavailable")
    -- Only the immediate value 0x04 is changed to 0x01.
    -- This does not change the adjacent monster count or change executable flow.
    local result=writeBytes(target+3,0x01)
    assert(result~=false,"CE rejected process write")
    local after=readBytes(target,#ORIGINAL,true)
    if not same(after,PATCHED) then
      -- Conditional rollback; never blindly overwrite code not written by us.
      if after and after[4]==0x01
          and after[11]==0x01 then pcall(writeBytes,target+3,0x04) end
      error("Patch verification failed; verify bytes manually")
    end
    state.pid,state.base,state.active=pid,base,true
    print("[Arena native slots] APPLIED. desired merc=1, monster=1")
    print("Launch LOCAL Arena now. This does NOT spawn a second monster.")
    print("After returning to menu: arenaNativeRestoreSlots()")
  end)
end
function arenaNativeRestoreSlots()
  withError("restore",function()
    assert(state.active,"No patch was applied by this script instance")
    local pid,base,target,observed=getSession()
    assert(state.pid==pid and state.base==base,
      "Evolve process changed; never restore into a different process")
    assert(same(observed,PATCHED),
      "Unexpected code bytes: refusing blind rollback: "..dump(observed))
    local result=writeBytes(target+3,0x04)
    assert(result~=false,"Restore write failed")
    local after=readBytes(target,#ORIGINAL,true)
    assert(same(after,ORIGINAL),"RESTORE NOT VERIFIED; exit the game before another test")
    state.active=false
    print("[Arena native slots] RESTORED stock desired merc=4, monster=1")
  end)
end
print("Arena native slot experiment loaded (no writes yet).")
print("Run arenaNativeSlotStatus() at the main menu.")
