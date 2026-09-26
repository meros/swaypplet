---
name: Everything (voidtools)
slug: everything-voidtools
domain: launchers
kind: product
platform: Windows
vendor: voidtools (David Carpenter)
years: 2009–present
status: current
tags: [filename index, ntfs mft, usn journal, instant search, memory]
relevance: medium
---

## What it is
A filename-only search engine for Windows that reads the NTFS Master File Table directly and keeps up to date from the USN change journal. Results update per keystroke over millions of files.

## What was new
- Index names only, not content: about 5 s and 35 MB for 250,000 files, about 1 minute and 100 MB for a million.
- Change-journal tracking instead of rescans or per-directory watchers.
- An IPC/SDK that other launchers (Flow Launcher, PowerToys) call instead of indexing themselves.

## What went wrong / limits
- NTFS-specific; on other filesystems it falls back to slower scanning. No content search by default.

## Lessons for swaypplet
- Take "names only, from the filesystem's own change log": on Linux the equivalents are `plocate` (periodic) or fanotify on btrfs/ext4 (live). Do not crawl from the shell.

## Sources
- https://www.voidtools.com/faq/ — index time, memory, USN journal [verified]
