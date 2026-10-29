-- Stage 2 read-only Arena monster/merc slot probe for Cheat Engine Lua.
-- Build: Evolve.exe 3.6.1.2538. No writes, breakpoints, hooks or code injection.
-- Use in an offline Arena match, then call arenaSlotSnapshot("after-spawn").
local ENTITY_SYSTEM_RVA=0x28D58E8
local NAME_RESOLVER_RVA=0x28D59A8
local ENTITY_VTABLE_RVA=0x22DBC60
local NAMES={"PlayerSlot_spectator_00","PlayerSlot_merc_00",
  "PlayerSlot_merc_01","PlayerSlot_merc_02","PlayerSlot_merc_03",
  "PlayerSlot_monster_00","PlayerSlot_monster_01","team_merc","team_monster"}
local TARGETS={}
for _,n in ipairs(NAMES) do TARGETS[n]=true end
local function ptr(x) return type(x)=="number" and x>=0x10000 and x<0x7FFFFFFFFFFF end
local function rp(a)
  if not ptr(a) then return nil end
  local ok,x=pcall(readPointer,a)
  return ok and ptr(x) and x or nil
end
local function u32(a)
  if not ptr(a) then return nil end
  local ok,v=pcall(readInteger,a)
  return ok and type(v)=="number" and v%0x100000000 or nil
end
local function bytes(a,n)
  if not ptr(a) then return nil end
  local ok,b=pcall(readBytes,a,n,true)
  return ok and type(b)=="table" and #b==n and b or nil
end
local function u64(b,off)
  local v=0
  for i=7,0,-1 do v=v*256+b[off+i+1] end
  return v
end
local function xor32(a,b)
  local v,bit=0,1
  while a>0 or b>0 do
    if a%2~=b%2 then v=v+bit end
    a,b,bit=math.floor(a/2),math.floor(b/2),bit*2
  end
  return v
end
local function resolveName(e,idx,entries)
  local code=u32(e+0x124)
  if not code then return nil end
  local ix=code-0xE11
  if ix<0 or ix>=0x10000 then return nil end
  local enc=u32(idx+ix*4)
  if not enc then return nil end
  local value=xor32(enc,0x47FA)
  if value>=0x10000 then return nil end
  local entry=rp(entries+value*8)
  local stringPtr=entry and rp(entry)
  local b=stringPtr and bytes(stringPtr,48)
  if not b then return nil end
  local chars={}
  for i=1,#b do
    if b[i]==0 then return #chars>=3 and table.concat(chars) or nil end
    if b[i]<32 or b[i]>126 then return nil end
    chars[#chars+1]=string.char(b[i])
  end
end
local function fmt(x) return x and string.format("0x%X",x) or "none" end
local logfile=(os.getenv("TEMP") or ".").."\\EvolveArenaSlotProbe.log"
local function log(s)
  print(s)
  local f=io.open(logfile,"a")
  if f then f:write(os.date("[%Y-%m-%d %H:%M:%S] ")..s.."\n");f:close() end
end
local snapshots={}
function arenaSlotSnapshot(label)
  local ok,err=pcall(function()
    local pid=getProcessIDFromProcessName("Evolve.exe")
    assert(pid and pid~=0,"Game not running")
    if getOpenedProcessID()~=pid then openProcess(pid) end
    local base=getAddressSafe("Evolve.exe")
    assert(ptr(base),"Evolve base unavailable")
    local system=rp(base+ENTITY_SYSTEM_RVA)
    local tablePtr=system and rp(system+0xA8)
    local resolver=rp(base+NAME_RESOLVER_RVA)
    local idx=resolver and rp(resolver+0x08)
    local entries=resolver and rp(resolver+0x20)
    assert(tablePtr and idx and entries,"Verified entity/name tables not available")
    local found,live={},0
    for offset=0,65535,512 do
      local b=bytes(tablePtr+offset*8,4096)
      if b then
        for i=0,511 do
          local entity=u64(b,i*8)
          if ptr(entity) then
            local id=u32(entity+0x18)
            if id and id~=0 and id%65536==offset+i then
              live=live+1
              assert(live<=8192,"Bounded scan exceeded")
              if rp(entity)==base+ENTITY_VTABLE_RVA then
                local name=resolveName(entity,idx,entries)
                if TARGETS[name] then
                  found[name]={id=id,entity=entity,raw=bytes(entity,0x180)}
                end
              end
            end
          end
        end
      end
    end
    label=tostring(label or "arena")
    log("SNAPSHOT "..label.." pid="..pid.." liveEntities="..live)
    for _,name in ipairs(NAMES) do
      local e=found[name]
      log(name.." "..(e and ("id="..fmt(e.id).." entity="..fmt(e.entity)) or "not resolved"))
    end
    local previous=snapshots[#snapshots]
    if previous and previous.pid==pid then
      for _,name in ipairs(NAMES) do
        local a,b=previous.found[name],found[name]
        if a and b and a.id==b.id and a.entity==b.entity and a.raw and b.raw then
          local diffs={}
          for i=1,#a.raw do
            if a.raw[i]~=b.raw[i] then
              diffs[#diffs+1]=string.format("%03X:%02X>%02X",i-1,a.raw[i],b.raw[i])
            end
          end
          if #diffs>0 then
            log("CHANGED "..name.." ("..#diffs.." bytes): "..table.concat(diffs," ",1,math.min(#diffs,35)))
          end
        end
      end
    end
    snapshots[#snapshots+1]={pid=pid,found=found}
  end)
  if not ok then log("SNAPSHOT ERROR: "..tostring(err)) end
end
log("Arena slot probe READY. Log: "..logfile)
log("Use arenaSlotSnapshot('in-arena') after match loads; repeat after spawn.")
