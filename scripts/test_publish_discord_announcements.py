"""No-network tests for Discord announcement extraction and link validation."""
import unittest

from publish_discord_announcements import clean_posts, discord_jump, safe_image_url

DEST = "123456789012345678"
GUILD = "223456789012345678"
SOURCE_GUILD = "323456789012345678"
SOURCE_CHANNEL = "423456789012345678"
SOURCE_MESSAGE = "523456789012345678"


def forward(message_id="623456789012345678", **overrides):
    message = {
        "id": message_id,
        "webhook_id": "723456789012345678",
        "channel_id": DEST,
        "guild_id": GUILD,
        "author": {"username": "Announcements"},
        "content": "Published update",
        "timestamp": "2026-10-25T12:00:00+00:00",
        "message_reference": {
            "guild_id": SOURCE_GUILD,
            "channel_id": SOURCE_CHANNEL,
            "message_id": SOURCE_MESSAGE,
        },
        "embeds": [],
    }
    message.update(overrides)
    return message


class FeedTests(unittest.TestCase):
    def test_only_forwarded_webhooks_are_published(self):
        messages = [forward(webhook_id=None), forward(channel_id=SOURCE_CHANNEL), forward()]
        posts = clean_posts(messages, DEST)
        self.assertEqual(len(posts), 1)
        self.assertEqual(posts[0]["content"], "Published update")

    def test_original_link_preferred_when_provided(self):
        post = clean_posts([forward()], DEST)[0]
        self.assertEqual(
            post["url"],
            f"https://discord.com/channels/{SOURCE_GUILD}/{SOURCE_CHANNEL}/{SOURCE_MESSAGE}",
        )

    def test_destination_link_is_fallback(self):
        post = clean_posts([forward(message_reference={})], DEST)[0]
        self.assertEqual(post["url"], f"https://discord.com/channels/{GUILD}/{DEST}/623456789012345678")

    def test_discard_arbitrary_media_links_and_excess_text(self):
        image = "https://cdn.discordapp.com/attachments/123/456/image.png"
        post = clean_posts([forward(
            content="x" * 7000,
            embeds=[{"title": "Update", "description": "Details",
                     "image": {"url": "https://attacker.example/image.png"}}],
            attachments=[{"url": image}, {"url": "http://cdn.discordapp.com/attachments/bad"}],
        )], DEST)[0]
        self.assertEqual(post["title"], "Update")
        self.assertEqual(post["description"], "Details")
        self.assertEqual(post["image_urls"], [image])
        self.assertEqual(len(post["content"]), 6000)

    def test_safe_links_reject_credentials_and_untrusted_host(self):
        self.assertIsNone(safe_image_url("https://cdn.discordapp.com.evil.org/attachments/1"))
        self.assertIsNone(safe_image_url("https://alice:pass@cdn.discordapp.com/attachments/1"))
        self.assertIsNone(discord_jump(GUILD, DEST, "not-a-snowflake"))

    def test_hard_limit_and_empty_or_malformed_posts(self):
        many = [forward(str(623456789012345678 + idx)) for idx in range(80)]
        self.assertEqual(len(clean_posts(many, DEST)), 20)
        self.assertEqual(clean_posts([forward(content="", embeds=[])], DEST), [])
        with self.assertRaises(ValueError):
            clean_posts({"not": "a list"}, DEST)


if __name__ == "__main__":
    unittest.main()
