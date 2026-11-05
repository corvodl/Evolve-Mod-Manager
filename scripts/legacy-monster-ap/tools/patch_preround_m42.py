"""On m41: supply selected-monster AP definitions from Stage 2 ability XML."""
import sys

path, generated = sys.argv[1:3]
src = open(path, encoding="utf-8").read()


def sub(old, new):
    global src
    if src.count(old) != 1:
        raise SystemExit("anchor not found once: " + old[:80])
    src = src.replace(old, new)


def subn(old, new, count):
    global src
    if src.count(old) != count:
        raise SystemExit("anchor not found %d times: %s" % (count, old[:80]))
    src = src.replace(old, new)


sub('''   // Stage 2's own MonsterAbilitySelect calls GetAbilities without a subtype.
   // The selected monster is already held by the native preround state.
   flash.external.ExternalInterface.call("GetAbilities");
   s2_log("abilities: GetAbilities (native selection " + m_my_subtype + ") -> AbilityInfo " + (AbilityInfo == undefined ? "undefined" : AbilityInfo.length + " values"));''', '''   // PreRound's native handler requires the monster subtype; the four-slot
   // definitions below come from the same Stage 2 ability XML that gameplay
    // uses when the native response is empty in the stage context. Discard
    // the previous monster's fallback first: the native call may do nothing.
    AbilityInfo = new Array();
    flash.external.ExternalInterface.call("GetAbilities",m_my_subtype);
   s2_log("abilities: native " + m_my_subtype + " -> " + (AbilityInfo == undefined ? "undefined" : AbilityInfo.length + " values"));
   s2_localAbilityInfo = AbilityInfo == undefined || AbilityInfo.length == 0;
   if(s2_localAbilityInfo)
   {
      AbilityInfo = s2_abilityInfoForMonster(m_my_subtype);
      s2_log("abilities: Stage 2 definitions for " + m_my_subtype + " -> " + AbilityInfo.length + " values");
   }''')

# The first entry of a slot has no earlier entry to inherit description arrays
# from, so both parse loops must tolerate an empty m_ability_data slot.
subn('''      if(_loc2_.desc != undefined)''', '''      if(_loc2_ != undefined && _loc2_.desc != undefined)''', 2)
subn('''      else if(_loc2_.skill_level != undefined)''', '''      else if(_loc2_ != undefined && _loc2_.skill_level != undefined)''', 2)

sub('''      if(prev_menu_index < PANE_ABILITY_SELECTION && s2_nativeAP != undefined && !isNaN(Number(s2_nativeAP)) && Number(s2_nativeAP) >= 0)
      {
         setAbilityPoints(Number(s2_nativeAP));
      }''', '''      if(prev_menu_index < PANE_ABILITY_SELECTION)
      {
         // The native AP getter belongs to the stock element and returns
         // undefined from PreRound. Initial character select spends four AP,
         // but only when the local four-slot definitions actually loaded.
         var s2_points;
         if(s2_nativeAP != undefined && !isNaN(Number(s2_nativeAP)) && Number(s2_nativeAP) >= 0)
         {
            s2_points = Number(s2_nativeAP);
            s2_log("abilities: " + m_my_subtype + " native budget " + s2_points);
         }
         else if(s2_localAbilityInfo == true && AbilityInfo != undefined && AbilityInfo.length > 0)
         {
            s2_points = 4;
            s2_log("abilities: " + m_my_subtype + " local budget " + s2_points);
         }
         if(s2_points != undefined)
         {
            setAbilityPoints(s2_points);
         }
      }''')

sub('''      flash.external.ExternalInterface.call("AcceptAbilities",_loc5_,_loc4_,_loc3_,_loc2_);''', '''      s2_log("abilities: submitting " + m_my_subtype + " -> " + [_loc5_,_loc4_,_loc3_,_loc2_].join(","));
      flash.external.ExternalInterface.call("AcceptAbilities",_loc5_,_loc4_,_loc3_,_loc2_);''')

src += "\n" + open(generated, encoding="utf-8").read()
open(path, "w", encoding="utf-8").write(src)
print("patched m42: four-point selected-monster ability data and submission logs")
