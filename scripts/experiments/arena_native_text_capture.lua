-- Evolve Stage 2 native code capture (READ-ONLY; optional full code sections).
-- Unlike crash minidumps which often omit code pages, this reads the live,
-- unpacked PE executable's mapped executable sections in 64 KiB chunks.
-- This does not change game memory, hook, inject, or call native game code.
--
-- Execute from Cheat Engine Lua Engine with Evolve.exe running at main menu.
-- Output EvolveArenaText_<section>_<RVA>.bin and manifest in %TEMP%.
-- Keep game code captures local/private; do not add game bytes to public PRs.

local SLOT_RVA=0x15CF820
local STOCK={0xC7,0x41,0x08,0x04,0x00,0x00,0x00,
             0xC7,0x41,0x0C,0x01,0x00,0x00,0x00}
local ONE_MERC={0xC7,0x41,0x08,0x01,0x00,0x00,0x00,
                0xC7,0x41,0x0C,0x01,0x00,0x00,0x00}
local MAX_SECTION=0x4000000  -- 64 MiB each
local MAX_TOTAL=0x6000000    -- 96 MiB total
local CHUNK=0x10000
local function same(a,b)
  if not a or #a~=#b then return false end
  for i=1,#b do if a[i]~=b[i] then return false end end
  return true
end
local function bytes(at,n)
  local ok,v=pcall(readBytes,at,n,true)
  if ok and type(v)=="table" and #v==n then return v end
end
local function uint16(b,idx)
  return b[idx]+b[idx+1]*256
end
local function uint32(b,idx)
  return b[idx]+b[idx+1]*256+b[idx+2]*65536+b[idx+3]*16777216
end
local function name(b)
  local s={}
  for i=1,8 do
    if b[i]==0 then break end
    if b[i]<32 or b[i]>126 then return nil end
    s[#s+1]=string.char(b[i])
  end
  return table.concat(s)
end
local function toBlob(v)
  local s={}
  for i=1,#v do s[i]=string.char(v[i]) end
  return table.concat(s)
end
local function capture()
  local pid=getProcessIDFromProcessName("Evolve.exe")
  assert(pid and pid~=0,"Evolve.exe is not running")
  if getOpenedProcessID()~=pid then openProcess(pid) end
  assert(getOpenedProcessID()==pid,"Process attach failed")
  local base=getAddressSafe("Evolve.exe")
  assert(type(base)=="number" and base>=0x10000,"No module base")
  local observed=bytes(base+SLOT_RVA,14)
  assert(same(observed,STOCK) or same(observed,ONE_MERC),
    "Not the known Stage 2 build / another live code patch is present")
  local dos=bytes(base,0x40)
  assert(dos and dos[1]==0x4D and dos[2]==0x5A,"Bad MZ header")
  local peOff=uint32(dos,0x3C+1)
  assert(peOff>=0x40 and peOff<0x400,"Bad PE offset")
  local nt=bytes(base+peOff,0x108)
  assert(nt and same({nt[1],nt[2],nt[3],nt[4]},{0x50,0x45,0,0}),"Bad PE signature")
  local nsects=uint16(nt,7)
  local optSize=uint16(nt,21)
  assert(nsects>=1 and nsects<=24 and optSize>=0xF0 and optSize<0x400,"Unsupported PE header")
  assert(uint16(nt,25)==0x20B,"Expected PE32+ executable")
  local imageSize=uint32(nt,24+56+1)
  assert(imageSize>=0x100000 and imageSize<=0x10000000,"Image size not plausible")
  local tmp=os.getenv("TEMP")
  assert(tmp and #tmp>2,"Missing TEMP directory")
  local outputBase=tmp.."\\EvolveArenaText_"
  local lines={
    "READ-ONLY LIVE CODE SECTIONS (Evolve.exe)",
    string.format("PID=%d BASE=0x%X SizeOfImage=0x%X",pid,base,imageSize),
    "RVA and VirtualSize are taken from live PE headers.",
    "No module or key material is modified.",
  }
  local total=0
  local count=0
  for i=0,nsects-1 do
    local sh=bytes(base+peOff+24+optSize+i*40,40)
    assert(sh,"Cannot read section header")
    local label=name(sh)
    local vs=uint32(sh,9)
    local rva=uint32(sh,13)
    local flags=uint32(sh,37)
    -- IMAGE_SCN_MEM_EXECUTE = 0x20000000
    local executable=math.floor(flags/0x20000000)%2==1
    if executable and vs>0 then
      assert(label and #label>0 and not label:find("[^%w_.]"),"Bad section name")
      assert(vs<=MAX_SECTION and total+vs<=MAX_TOTAL,"Executable sections exceed bounded budget")
      assert(rva>=0x1000 and rva+vs<=imageSize,"Section outside PE image")
      total=total+vs
      local target=string.format("%s%s_%08X.bin",outputBase,label,rva)
      local f=assert(io.open(target,"wb"))
      local written=0
      while written<vs do
        local size=math.min(CHUNK,vs-written)
        local block=bytes(base+rva+written,size)
        if not block then
          f:close()
          os.remove(target)
          error(string.format("Cannot read section %s at RVA=0x%X",label,rva+written))
        end
        assert(f:write(toBlob(block)),"Disk write failed")
        written=written+size
      end
      f:close()
      count=count+1
      local summary=string.format("section=%s rva=0x%X len=0x%X file=%s",label,rva,vs,target)
      lines[#lines+1]=summary
      print(summary)
    end
  end
  assert(count>0,"No executable sections captured")
  local manifest=outputBase.."manifest.txt"
  local f=assert(io.open(manifest,"wb"))
  assert(f:write(table.concat(lines,"\r\n").."\r\n"))
  f:close()
  print(string.format("Captured %d executable sections (%d bytes) to TEMP.",count,total))
  print("Manifest: "..manifest)
  print("These are local code captures, NOT safe-to-call engine function signatures.")
end
local ok,err=pcall(capture)
if not ok then print("[Arena PE capture] FAILED: "..tostring(err)) end
