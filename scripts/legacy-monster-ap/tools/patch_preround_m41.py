"""On m40: undo AP spending one point at a time, scoped to a monster pick."""
import sys

path = sys.argv[1]
src = open(path, encoding="utf-8").read()


def sub(old, new):
    global src
    if src.count(old) != 1:
        raise SystemExit("anchor not found once: " + old[:70])
    src = src.replace(old, new)


sub('''function parseAbilityData()
{
   m_ability_data = new Array(NUM_ABILITIES_ALLOWED);''', '''function parseAbilityData()
{
   // A new monster starts a new AP budget and undo history. Returning from
   // perks to the same monster does not re-enter this parser.
   s2_abilityMonster = String(m_my_subtype);
   s2_abilityHistory = [];
   m_ability_data = new Array(NUM_ABILITIES_ALLOWED);''')

sub('''         setAbilityPoints(Number(s2_nativeAP));
      }
      clearAbilities(false,true);''', '''         setAbilityPoints(Number(s2_nativeAP));
      }
      if(prev_menu_index < PANE_ABILITY_SELECTION)
      {
         clearAbilities(false,true);
      }''')
sub('''      if(s2_nativeAP != undefined && !isNaN(Number(s2_nativeAP)) && Number(s2_nativeAP) >= 0)''', '''      if(prev_menu_index < PANE_ABILITY_SELECTION && s2_nativeAP != undefined && !isNaN(Number(s2_nativeAP)) && Number(s2_nativeAP) >= 0)''')

sub('''function reduceAbilitySelection(btn)
{
   if(m_num_ability_points >= m_max_ability_points || btn.skill_level <= 0)
   {
      return undefined;
   }
   m_num_ability_points++;''', '''function reduceAbilitySelection(btn, skipHistory)
{
   if(m_num_ability_points >= m_max_ability_points || btn.skill_level <= 0)
   {
      return undefined;
   }
   if(skipHistory != true && s2_abilityHistory != undefined)
   {
      for(var s2_hi = s2_abilityHistory.length - 1; s2_hi >= 0; s2_hi--)
      {
         if(s2_abilityHistory[s2_hi] == btn.index)
         {
            s2_abilityHistory.splice(s2_hi,1);
            break;
         }
      }
   }
   m_num_ability_points++;''')

sub('''   m_num_ability_points--;
   btn.skill_level += 1;
   updateAbilityText();''', '''   m_num_ability_points--;
   btn.skill_level += 1;
   if(s2_abilityHistory == undefined)
   {
      s2_abilityHistory = [];
   }
   s2_abilityHistory.push(btn.index);
   s2_log("abilities: spend slot " + (btn.index + 1) + " level " + btn.skill_level + ", remaining " + m_num_ability_points);
   updateAbilityText();''')

sub('''function clearAbilities(force, skipAutoFocus)
{
   Selection.setFocus(undefined);
   if(force == true || selection_pane.ability_menu.respec_btn.enabled)
   {
      resetAbilityScreenState(true);
      if(skipAutoFocus != true)
      {
         autoFocusAbilityButton();
      }
   }
}''', '''function clearAbilities(force, skipAutoFocus)
{
   Selection.setFocus(undefined);
   if(force == true)
   {
      s2_abilityHistory = [];
      resetAbilityScreenState(true);
   }
   else if(skipAutoFocus == true)
   {
      // Pane entry/reparse: do not consume an undo step. No native data means
      // the buttons must not be initialised from undefined abilities.
      if(m_ability_data != undefined && m_ability_data[0] != undefined && m_max_ability_points != undefined)
      {
         resetAbilityScreenState(false);
      }
   }
   else if(selection_pane.ability_menu.respec_btn.enabled && s2_abilityHistory != undefined && s2_abilityHistory.length > 0)
   {
      var s2_slot = s2_abilityHistory.pop();
      var s2_btn = selection_pane.ability_menu["btn" + (s2_slot + 1)];
      if(s2_btn != undefined)
      {
         reduceAbilitySelection(s2_btn,true);
         s2_log("abilities: undo slot " + (s2_slot + 1) + ", remaining " + m_num_ability_points);
      }
   }
   if(skipAutoFocus != true && m_ability_data != undefined && m_ability_data[0] != undefined)
   {
      autoFocusAbilityButton();
   }
}''')

open(path, "w", encoding="utf-8").write(src)
print("patched m41: one-step AP undo and monster-scoped history")
