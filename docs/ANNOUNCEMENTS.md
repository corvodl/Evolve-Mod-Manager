# What's New: latest version notes + forwarded Discord announcements

The manager's **What's New** screen has independent sections for the newest
published GitHub Release (including Nightly prereleases) and the latest 20
forwarded Discord announcements. The UI refreshes at startup, when the page
is reopened after ten minutes, and every ten minutes while it remains open.
The last successful data is cached in the per-user manager settings folder.
Unreachable feeds never prevent running the manager, updating, or playing.

## Configure announcements (repo owner, once)

1. Follow the other community's **announcement channel** into a dedicated text
   channel on a Discord server you manage. Only posts the source server
   explicitly *publishes* are forwarded. Previously published posts are not
   retroactively forwarded.
2. In the Discord Developer Portal create an app/bot, install it **only in your
   own server**, grant **View Channel** and **Read Message History** to the
   destination channel, and enable the **Message Content Intent** if applicable.
   No ability to send, delete or moderate messages is needed.
3. Under **GitHub → Settings → Secrets and variables → Actions → Repository
   secrets**, add **both** `DISCORD_BOT_TOKEN` (bot token, secret) and
   `DISCORD_CHANNEL_ID` (destination channel snowflake, also accepted as a
   secret). Do not place either in code, a release, an issue, or a log.
4. Under **GitHub → Settings → Pages**, select **Build and deployment →
   Source: GitHub Actions**. GitHub Pages is required for this deployment flow.
5. After the PR is merged, open **Actions → Publish Discord announcement feed →
   Run workflow** on `main`. It also runs on relevant pushes and approximately
   every 30 minutes (scheduled workflows can be delayed by GitHub). Check the
   published URL:
   `https://corvodl.github.io/Evolve-Mod-Manager/announcements.json`.

The workflow requests messages from Discord *inside GitHub Actions* using the
bot token, selects only webhook-authored (forwarded) messages from the exact
configured channel, and publishes a small, bounded, text-focused JSON result.
Tokens are **never** included in that result or shipped inside the desktop.
No long-running bot, Discord sign-in or in-app credentials are needed.

The destination channel must contain genuinely forwarded announcements.
If the bot has no access, has an invalid token or lacks message-content access,
the workflow fails with a non-secret diagnostic or publishes zero announcements.
Discord API message/attachment access may require privileged-intent approval
for verified bots.

## Manager display

- GitHub Releases API: newest non-draft *published* release, including nightlies.
  This is presentation only; the separately authenticated update installer
  continues to verify actual release assets and digests.
- Discord JSON feed: last 20 forwarded posts, newest first, plain-text body,
  embed title/description, author, timestamp, links to the source or destination
  Discord message and up to four attachment-image **links**. Images are linked
  rather than downloaded or embedded; Discord media URLs may expire.
- Each section has separate errors and retains cached content if offline.
  Opening links requires a user click and only approved HTTPS GitHub/Discord
  URLs are accepted. Release bodies display an excerpt with a link to the
  full GitHub release.

**Permission reminder:** Following another server's Discord announcement
channel does not automatically grant permission to republish its content on
a public website. Confirm public redistribution is acceptable to the
original community before enabling the workflow.

## Testing

Run without credentials:

```bash
python3 -m unittest discover -s scripts -p 'test_publish_discord_announcements.py'
cargo fmt --all -- --check
cargo test -p evolve-mod-manager whats_new
```

Real setup testing still needs the configured GitHub Actions secrets, Pages,
one newly published forwarded message and a Windows manager build.
