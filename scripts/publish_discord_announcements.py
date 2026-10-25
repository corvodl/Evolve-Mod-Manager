#!/usr/bin/env python3
"""Publish a read-only, bounded JSON feed of Discord-followed announcement posts.

The bot token is read only from the GitHub Actions secret environment. Never
write the token, raw Discord response headers or credentials to the feed.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import sys
import urllib.error
import urllib.request
from datetime import datetime, timezone
from urllib.parse import urlsplit

CHANNEL_ID = re.compile(r"^[0-9]{15,22}$")
MAX_API_BYTES = 2 * 1024 * 1024
MAX_POSTS = 20
MAX_TEXT = 6000
IMAGE_HOSTS = {"cdn.discordapp.com", "media.discordapp.net"}


def safe_image_url(url: object) -> str | None:
    """Do not forward arbitrary external or non-TLS links into a trusted feed."""
    if not isinstance(url, str) or len(url) > 2048:
        return None
    try:
        parts = urlsplit(url)
    except ValueError:
        return None
    if parts.scheme != "https" or parts.hostname not in IMAGE_HOSTS:
        return None
    if parts.username or parts.password or not parts.path.startswith("/attachments/"):
        return None
    return url


def discord_jump(guild: object, channel: object, message: object) -> str | None:
    ids = [guild, channel, message]
    if all(isinstance(value, str) and CHANNEL_ID.fullmatch(value) for value in ids):
        return "https://discord.com/channels/" + "/".join(ids)
    return None


def compact_text(value: object, limit: int = MAX_TEXT) -> str:
    if not isinstance(value, str):
        return ""
    return "".join(ch for ch in value if ch == "\n" or ch == "\t" or ord(ch) >= 32)[:limit].strip()


def clean_posts(messages: object, destination_id: str) -> list[dict]:
    if not isinstance(messages, list):
        raise ValueError("Discord did not return a message list")
    posts = []
    for message in messages:
        if not isinstance(message, dict):
            continue
        # Channel Following creates webhook-authored posts. Do not republish
        # ordinary community messages from this channel by accident.
        if not message.get("webhook_id") or not isinstance(message.get("id"), str):
            continue
        if message.get("channel_id") != destination_id:
            continue
        forward = message.get("message_reference") or {}
        if not isinstance(forward, dict):
            forward = {}
        local = discord_jump(message.get("guild_id"), destination_id, message["id"])
        original = discord_jump(
            forward.get("guild_id"), forward.get("channel_id"), forward.get("message_id")
        )
        link = original or local
        if not link:
            continue
        embeds = message.get("embeds") if isinstance(message.get("embeds"), list) else []
        content = compact_text(message.get("content"))
        title = ""
        details = []
        images = []
        for embed in embeds[:5]:
            if not isinstance(embed, dict):
                continue
            if not title:
                title = compact_text(embed.get("title"), 200)
            desc = compact_text(embed.get("description"), 2000)
            if desc:
                details.append(desc)
            for tag in ("image", "thumbnail"):
                image = embed.get(tag)
                url = safe_image_url(image.get("url")) if isinstance(image, dict) else None
                if url and url not in images:
                    images.append(url)
        attachments = message.get("attachments")
        if isinstance(attachments, list):
            for item in attachments[:8]:
                if not isinstance(item, dict):
                    continue
                url = safe_image_url(item.get("url"))
                if url and url not in images:
                    images.append(url)
        author = message.get("author")
        author_name = compact_text(author.get("username"), 80) if isinstance(author, dict) else ""
        date = compact_text(message.get("timestamp"), 40)
        if not content and not details and not title and not images:
            continue
        posts.append({
            "id": message["id"],
            "author": author_name or "Discord announcement",
            "timestamp": date,
            "title": title,
            "content": content,
            "description": "\n\n".join(details)[:MAX_TEXT],
            "url": link,
            "image_urls": images[:4],
        })
        if len(posts) >= MAX_POSTS:
            break
    return posts


def get_messages(token: str, channel: str) -> object:
    request = urllib.request.Request(
        f"https://discord.com/api/v10/channels/{channel}/messages?limit=100",
        headers={
            "Authorization": "Bot " + token,
            "User-Agent": "Evolve-Mod-Manager-Announcements/1.0",
            "Accept": "application/json",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            data = response.read(MAX_API_BYTES + 1)
    except urllib.error.HTTPError as error:
        raise RuntimeError(
            f"Discord returned HTTP {error.code}; verify token, Message Content Intent "
            "and View Channel / Read Message History permissions."
        ) from None
    if len(data) > MAX_API_BYTES:
        raise RuntimeError("Discord response exceeded the allowed feed size")
    return json.loads(data)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    token = os.environ.get("DISCORD_BOT_TOKEN", "").strip()
    channel = os.environ.get("DISCORD_CHANNEL_ID", "").strip()
    if not token or not CHANNEL_ID.fullmatch(channel):
        print(
            "Missing DISCORD_BOT_TOKEN or invalid DISCORD_CHANNEL_ID. "
            "Both are required repository Actions secrets.",
            file=sys.stderr,
        )
        return 2
    try:
        posts = clean_posts(get_messages(token, channel), channel)
        result = {
            "schema_version": 1,
            "updated_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
            "announcements": posts,
        }
        args.output.parent.mkdir(parents=True, exist_ok=True)
        temporary = args.output.with_suffix(".json.tmp")
        temporary.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        temporary.replace(args.output)
    except (RuntimeError, ValueError, OSError, json.JSONDecodeError) as error:
        print(f"Announcements feed not published: {error}", file=sys.stderr)
        return 1
    print(f"Published {len(posts)} forwarded Discord announcements")
    if not posts:
        print("No webhook-forwarded posts found. Confirm the source channel was followed and its posts published.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
