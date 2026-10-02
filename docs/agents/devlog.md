# Devlog entries

The gm devlog lives on Jeff's site, in the portfolio repo at
`content/work/gm.md`. When Jeff asks for a devlog entry ("log this", "write a
devlog entry"), do this:

1. **Read the log and the voice guide.**
   ```sh
   gh api repos/Jefferson-Butler1/portfolio/contents/content/work/gm.md --jq .content | base64 -d
   gh api repos/Jefferson-Butler1/portfolio/contents/content/VOICE.md --jq .content | base64 -d
   ```
2. **Find what's new.** The last entry's heading has its date range. List what
   landed since then:
   ```sh
   gh pr list --repo Jefferson-Butler1/gm --state merged --search "merged:>=YYYY-MM-DD" --json number,title,url
   gh issue list --repo Jefferson-Butler1/gm --state closed --search "closed:>=YYYY-MM-DD" --json number,title,url
   gh pr list --repo Jefferson-Butler1/gm --state open --json number,title,url,headRefName
   ```
   Open PRs count too: Jeff playtests builds before they merge to `master`.
   One entry covers one theme. If the theme isn't obvious, ask Jeff.
3. **Draft the entry**, following `VOICE.md`:
   - Append it at the end (entries are oldest-first), with a heading like
     `## Floors that can't fail (Sept 25–27)`.
   - Link every factual claim to its source: the gm issue, PR, or file where
     it's recorded, or the external primary source. Link gm files at a commit
     SHA (`gm/blob/<sha>/path`), not `master`, so old entries don't rot.
   - Describe only work that's built. Quote measurements from where they're
     recorded.
   - Update `## Current state` and its "Updated" date.
4. **Open a PR on the portfolio repo.** The PR is the review: Jeff edits or
   approves the copy there, and merging deploys the site.
   ```sh
   tmp=$(mktemp -d)
   gh repo clone Jefferson-Butler1/portfolio "$tmp" -- --depth 1
   git -C "$tmp" switch -c devlog/YYYY-MM-DD-<slug>
   # write the updated content/work/gm.md into "$tmp"
   git -C "$tmp" commit -am "devlog: <entry title>"
   git -C "$tmp" push -u origin HEAD
   gh pr create --repo Jefferson-Butler1/portfolio --head devlog/YYYY-MM-DD-<slug> \
     --title "devlog: <entry title>" --body "New gm devlog entry: <one line>."
   ```
