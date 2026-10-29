-- Evolve Stage 2: Monster Duel / AI-minion preflight (READ ONLY).
-- Cheat Engine > Memory View > Tools > Lua Engine > Execute once.
-- This does NOT spawn anything, edit memory, inject code or hook game functions.
-- Build-specific addresses are borrowed from runtime_actors.rs (Oct 2026).
-- Tests whether an existing Defend / "Second Monster" minion has a normal
-- actor backlink, registered entity name, health, armor and stage fields.

local ENTITY_SYSTEM_RVA = 0x28D58E8
local NAME_RESOLVER_RVA = 0x28D59A8
local ENTITY_VT = 0x22DBC60
local MAX_SLOTS = 65536
local MAX_LIVE_ENTITIES = 8192
local MAX_ACTORS = 256
local MAX_DISPLAY_ACTORS = 70
local MAX_DISPLAY_NAMES = 40
local samples = {}

local function hx(v) return string.format("0x%X", v) end
local function pointerOk(v)
  return type(v) == "number" and v >= 0x10000 and v < 0x7FFFFFFFFFFF
end
local function rp(addr)
  if not pointerOk(addr) then return nil end
  local ok, v = pcall(readPointer, addr)
  if ok and pointerOk(v) then return v end
  return nil
end
local function need(addr, label)
  local v = rp(addr)
  assert(v, "Missing " .. label .. " pointer at " .. hx(addr))
  return v
end
local function u32(addr)
  local ok, v = pcall(readInteger, addr)
  if not ok or type(v) ~= "number" then return nil end
  return v % 0x100000000
end
local function float(addr)
  local ok, v = pcall(readFloat, addr)
  if not ok or type(v) ~= "number" or v ~= v
      or v == math.huge or v == -math.huge then return nil end
  return v
end
local function bytes(addr, size)
  if not pointerOk(addr) then return nil end
  local ok, b = pcall(readBytes, addr, size, true)
  if ok and type(b) == "table" and #b == size then return b end
  return nil
end
local function u64Bytes(b, offset)
  -- offset is zero-based within a Cheat Engine byte table.
  local p = 0
  for i = 7, 0, -1 do p = p * 256 + b[offset+i+1] end
  return p
end
local function xor32(a, b)
  local value, place = 0, 1
  while a > 0 or b > 0 do
    if (a % 2) ~= (b % 2) then value = value + place end
    a, b, place = math.floor(a/2), math.floor(b/2), place*2
  end
  return value
end
local function position(addr)
  local x, y, z = float(addr + 0x40), float(addr + 0x44), float(addr + 0x48)
  if not x or not y or not z or math.abs(x)>100000 or
     math.abs(y)>100000 or math.abs(z)>100000 then return nil end
  return {x,y,z}
end
local function formatPosition(p)
  if not p then return "unknown" end
  return string.format("%.1f, %.1f, %.1f", p[1],p[2],p[3])
end

local function resolve()
  local pid = getProcessIDFromProcessName("Evolve.exe")
  assert(pid and pid ~= 0, "Evolve.exe is not running")
  if getOpenedProcessID() ~= pid then openProcess(pid) end
  assert(getOpenedProcessID() == pid, "Could not attach to Evolve.exe")
  local base = getAddressSafe("Evolve.exe")
  assert(pointerOk(base), "Evolve.exe module base not found")

  local anchor = base + ENTITY_SYSTEM_RVA
  local game = need(anchor - 0x10, "Game")
  local framework = need(game + 0x60, "Framework")
  local client = need(framework + 0x528, "Local client")
  local actor = need(client + 0x178, "Local actor")
  local id = u32(client + 0x170)
  assert(id and id ~= 0, "Enter a match with a spawned local actor")
  local system = need(anchor, "Entity system")
  local tableAddr = need(system + 0xA8, "Entity table")
  local localEntity = need(tableAddr + (id % 65536) * 8, "Local entity")
  assert(u32(localEntity + 0x18) == id, "Local entity ID mismatch")
  assert(rp(actor + 0x38) == localEntity, "Actor/entity mismatch")
  assert(rp(localEntity) == base + ENTITY_VT,
    "Entity vtable differs from verified Stage 2 build; aborting")

  return {pid=pid, base=base, id=id, localActor=actor,
          localEntity=localEntity, tableAddr=tableAddr,
          framework=framework, system=system}
end

local function findBacklink(localEntity, localActor)
  local function pointers(obj)
    local found = {}
    for offset = 0, 0x3F8, 8 do
      local p = rp(obj + offset)
      if p then found[#found+1] = {offset, p} end
    end
    return found
  end
  local children = pointers(localEntity)
  for _, t in ipairs(children) do
    if t[2] == localActor then
      return {kind="direct", a=t[1]}
    end
  end
  for index=1,math.min(#children,80) do
    local first = children[index]
    local sub = pointers(first[2])
    for _, second in ipairs(sub) do
      if second[2] == localActor then
        return {kind="indirect", a=first[1], b=second[1]}
      end
    end
  end
  return nil
end

local function actorForEntity(entity, route)
  if not route then return nil end
  local actor
  if route.kind == "direct" then
    actor = rp(entity + route.a)
  else
    local child = rp(entity + route.a)
    if child then actor = rp(child + route.b) end
  end
  if actor and rp(actor + 0x38) == entity then return actor end
  return nil
end

local function nameTables(base)
  local resolver = rp(base + NAME_RESOLVER_RVA)
  if not resolver then return nil end
  local idx = rp(resolver + 0x08)
  local entries = rp(resolver + 0x20)
  if idx and entries then return {idx=idx, entries=entries} end
  return nil
end

local function nameFromEntity(entity, tables)
  if not tables then return nil end
  local id = u32(entity + 0x124)
  if not id then return nil end
  local index = id - 0xE11
  if index < 0 or index >= 0x10000 then return nil end
  local enc = u32(tables.idx + index * 4)
  if not enc then return nil end
  local decoded = xor32(enc, 0x47FA)
  if decoded >= 0x10000 then return nil end
  local entry = rp(tables.entries + decoded * 8)
  local cstr = entry and rp(entry)
  if not cstr then return nil end
  local b = bytes(cstr, 32)
  if not b then return nil end
  local chars = {}
  for i=1,32 do
    if b[i] == 0 then
      if #chars < 3 then return nil end
      return table.concat(chars)
    end
    if b[i] < 33 or b[i] > 126 then return nil end
    chars[#chars+1] = string.char(b[i])
  end
  return nil
end

local function pair(actor, currentOff, maxOff)
  local a, b = float(actor+currentOff), float(actor+maxOff)
  if not a or not b or b <= 0 or b > 1000000 or
     a < 0 or a > 100000 then return nil end
  return string.format("%.1f/%.1f", a, b)
end
local function stage(actor)
  local v = u32(actor + 0x1260)
  if v == 0x0101 then return "1" end
  if v == 0x0202 then return "2" end
  if v == 0x0303 then return "3" end
  return "unverified"
end

local function looksLikeMonster(name)
  if not name then return false end
  local n = string.lower(name)
  for _, term in ipairs({
    "goliath","kraken","wraith","teleporter","gorgon",
    "behemoth","meteor","minion","monster","scorpid"
  }) do
    if string.find(n,term,1,true) then return true end
  end
  return false
end

local function snapshot(label)
  local r = resolve()
  local route = findBacklink(r.localEntity, r.localActor)
  local tables = nameTables(r.base)
  local allActors, namedEntities = {}, {}
  local liveCount, namedCount, totalActors = 0, 0, 0
  local overflow = false
  local localSeen = false

  for start = 0, MAX_SLOTS-1, 512 do
    if overflow then break end
    local b = bytes(r.tableAddr + start*8, 512*8)
    if b then
      for i=0,511 do
        local entity = u64Bytes(b,i*8)
        if pointerOk(entity) then
          local id = u32(entity + 0x18)
          if id and id ~= 0 and id % 65536 == start+i then
            liveCount = liveCount + 1
            if liveCount > MAX_LIVE_ENTITIES then
              overflow = true
              break
            end
            local isLocal = (id == r.id and entity == r.localEntity)
            if isLocal then localSeen = true end
            local actor = isLocal and r.localActor or actorForEntity(entity,route)
            local name = nameFromEntity(entity,tables)
            if name then namedCount=namedCount+1 end

            if actor then
              totalActors=totalActors+1
              if #allActors < MAX_ACTORS then
                allActors[#allActors+1] = {
                  id=id, entity=entity, actor=actor,
                  localActor=isLocal, name=name or "(unresolved)",
                  pos=position(entity),
                  health=pair(actor,0x198,0x1A0),
                  armor=pair(actor,0x1038,0x103C),
                  stage=stage(actor),
                  healthValue=float(actor+0x198),
                  armorValue=float(actor+0x1038)
                }
              end
            end
            if name and looksLikeMonster(name)
               and #namedEntities < MAX_DISPLAY_NAMES then
              namedEntities[#namedEntities+1] = {
                id=id,name=name,entity=entity,pos=position(entity),
                linkedActor=actor
              }
            end
          end
        end
      end
    end
  end

  -- The local actor is verified before enumeration; don't silently lose it.
  if not localSeen then
    error("Local actor disappeared during entity-table scan; retry")
  end
  print("\n================ "..label.." ================")
  print(string.format("PID %d | Base %s | Local actor ID %s",r.pid,hx(r.base),hx(r.id)))
  print(string.format("Local actor %s | Entity %s",hx(r.localActor),hx(r.localEntity)))
  if route then
    print("Actor backlink: "..route.kind.." entity+"..hx(route.a)
      ..(route.b and (" -> child+"..hx(route.b)) or ""))
  else
    print("Actor backlink NOT found; only local actor is guaranteed")
  end
  print(string.format("Live entities %d | named %d | actor matches %d | stored %d",
    liveCount,namedCount,totalActors,#allActors))
  if overflow then print("WARNING: live-entity cap reached; listing truncated") end

  table.sort(allActors,function(a,b)
    if a.localActor ~= b.localActor then return a.localActor end
    if looksLikeMonster(a.name) ~= looksLikeMonster(b.name) then
      return looksLikeMonster(a.name)
    end
    return a.id < b.id
  end)

  print("Actors (name is an ENTITY INSTANCE name, not verified class):")
  for i=1,math.min(#allActors,MAX_DISPLAY_ACTORS) do
    local a=allActors[i]
    print(string.format(
      "%s ID=%s name=%s actor=%s entity=%s stage=%s HP=%s armor=%s pos=[%s]",
      a.localActor and "[LOCAL]" or "[ACTOR]",hx(a.id),a.name,hx(a.actor),
      hx(a.entity),a.stage,a.health or "unknown",a.armor or "unknown",
      formatPosition(a.pos)))
  end
  print("Monster/minion-like ENTITY NAMES (can include non-actors):")
  for _, a in ipairs(namedEntities) do
    print(string.format("ID=%s name=%s entity=%s actor=%s pos=[%s]",
      hx(a.id),a.name,hx(a.entity),
      a.linkedActor and hx(a.linkedActor) or "not established",
      formatPosition(a.pos)))
  end
  return {label=label,pid=r.pid,localId=r.id,
          actors=allActors,monsterNames=namedEntities,
          liveCount=liveCount,actorCount=totalActors,
          routeFound=route~=nil}
end

local function compare(a,b)
  print("\n--- Comparison: "..a.label.." vs "..b.label.." ---")
  print(string.format("Actors: %d -> %d | live entities: %d -> %d",
    a.actorCount,b.actorCount,a.liveCount,b.liveCount))
  if a.pid ~= b.pid or a.localId ~= b.localId then
    print("Different process or local actor/session: numeric addresses and")
    print("entity IDs cannot be compared as the same actors across matches.")
    return
  end
  local earlier={}
  for _,actor in ipairs(a.actors) do earlier[actor.id]=actor end
  for _,actor in ipairs(b.actors) do
    local old=earlier[actor.id]
    if old and old.entity==actor.entity and old.actor==actor.actor then
      local dh=(actor.healthValue and old.healthValue)
        and (actor.healthValue-old.healthValue)
      local da=(actor.armorValue and old.armorValue)
        and (actor.armorValue-old.armorValue)
      if (dh and math.abs(dh)>0.01) or (da and math.abs(da)>0.01) then
        print(string.format("%s (%s) deltaHP=%s deltaArmor=%s",
          actor.name,hx(actor.id),
          dh and string.format("%+.2f",dh) or "unknown",
          da and string.format("%+.2f",da) or "unknown"))
      end
    end
  end
  print("No health change does not prove a hit was registered or rejected.")
end

local prompts = {
  {"A - Baseline", "Enter an offline match as a monster. Press OK to snapshot."},
  {"B - Second monster present", "If supported, start an offline match with the Second Monster map effect or Defend minions. Wait until the minion is visibly spawned, then press OK."},
  {"C - After attack", "While in the same match as B, attempt an ability/melee hit on the minion, or observe monster-minion combat. Press OK soon afterward."}
}

local ok,err=pcall(function()
  print("EVOLVE MONSTER DUEL PREFLIGHT | READ-ONLY")
  print("No Cheat Engine writes, hooks, injections or runtime engine calls.")
  for _, p in ipairs(prompts) do
    local label=inputQuery("Monster duel preflight",p[2]..
      "\n\nSwitch back to CE and click OK to capture."..
      "\nCancel ends the investigation.",p[1])
    if label==nil then break end
    samples[#samples+1]=snapshot(label)
  end
  if #samples>=2 then compare(samples[1],samples[2]) end
  if #samples>=3 then compare(samples[2],samples[3]) end
end)
if not ok then print("[PREFLIGHT ERROR] "..tostring(err)) end
print("\nMonster duel preflight finished. Copy the full Lua Engine output.")
