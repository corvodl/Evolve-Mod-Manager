"""Synthetic checks for read-only Legacy UI Data and artwork inventory audit."""
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest import TestCase, main
from zipfile import ZipFile
from audit_legacy_ui import audit, MAIN_SCREEN, MAIN_UI, NEWS


SCREEN = b"""<ScreenCinematics><MainMenu><Main>
<Button display_name="@ui_mp_OnlineGame" command="OnlineMultiplayer"/>
<Button display_name="@ui_Singleplayer" command="Singleplayer"/>
<Button display_name="@ui_Switchuser" command="SwitchUser" platform="durango"/>
<Button display_name="@ui_Options" command="GoToSettings" cinematic_event="open_options_menu"/>
</Main></MainMenu></ScreenCinematics>"""
ELEMENT = b"""<UIElements><UIElement name="MainMenuNew" mouseevents="1"
cursor="1" controller_input="1"><GFx file="main_menu.gfx" layer="3"/>
<functions><function name="OpenScreen" funcname="openScreen"/></functions>
<arrays><array name="menuButtons_names" varname="menuButtons_names"/></arrays>
</UIElement></UIElements>"""
NEWS_DATA = b"""<MenuNews><News text="Example" img="Libs/UI/FlashAssets/images/News/news_img1.dds"/></MenuNews>"""


class LegacyUiAuditTests(TestCase):
    def setUp(self):
        self.tmp = TemporaryDirectory()
        root = Path(self.tmp.name)
        self.data = root / "legacy-ui.zip"
        self.assets = root / "legacy-assets.zip"
        with ZipFile(self.data, "w") as z:
            z.writestr(MAIN_SCREEN, SCREEN)
            z.writestr(MAIN_UI, ELEMENT)
            z.writestr(NEWS, NEWS_DATA)
        with ZipFile(self.assets, "w") as z:
            z.writestr("files/Libs/UI/main_menu.gfx", b"GFX sample")
            z.writestr("files/Libs/UI/FlashAssets/images/News/news_img1.dds.0", b"DDS sample")

    def tearDown(self):
        self.tmp.cleanup()

    def test_preserves_pc_order_without_importing_console_actions(self):
        info = audit(self.data, self.assets)
        buttons = info["legacy_main_menu_pc_buttons"]
        self.assertEqual(
            [b["command"] for b in buttons],
            ["OnlineMultiplayer", "Singleplayer", "GoToSettings"],
        )
        self.assertIn("menuButtons_names", info["legacy_main_menu_ui_binding"]["dynamic_arrays"])
        self.assertIn("openScreen", info["legacy_main_menu_ui_binding"]["native_functions"])
        self.assertTrue(info["asset_inventory"]["Libs/UI/main_menu.gfx"])
        self.assertTrue(info["news_reference_images"]["Libs/UI/FlashAssets/images/News/news_img1.dds"])
        self.assertFalse(info["asset_inventory"]["Libs/UI/main_menu_I70.dds.0"])
        self.assertIn("Stage 2", info["intent"])

    def test_rejects_mismatched_gfx_binding(self):
        with ZipFile(self.data, "w") as z:
            z.writestr(MAIN_SCREEN, SCREEN)
            z.writestr(MAIN_UI, ELEMENT.replace(b"main_menu.gfx", b"other.gfx"))
            z.writestr(NEWS, NEWS_DATA)
        with self.assertRaisesRegex(ValueError, "binding"):
            audit(self.data)

    def test_rejects_xml_entity_and_oversized_payload(self):
        with ZipFile(self.data, "w") as z:
            z.writestr(MAIN_SCREEN, b'<!DOCTYPE x [<!ENTITY ex SYSTEM "x">]><ScreenCinematics/>')
            z.writestr(MAIN_UI, ELEMENT)
            z.writestr(NEWS, NEWS_DATA)
        with self.assertRaisesRegex(ValueError, "unsupported"):
            audit(self.data)


if __name__ == "__main__":
    main()
