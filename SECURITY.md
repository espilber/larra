# Security policy

## Supported versions

The current release is **0.9.3**.

## What Larra does with data

Larra is AGPL-3.0-or-later licensed and published by the Larra contributors. Inherited Rebost code remains MIT; see [NOTICE.md](NOTICE.md). There is no account to create.

Chat, reading Shelf documents, search, and answers all run on the machine where Larra is installed. Those documents and the installed AI stay on that machine.

Larra uses the network to **search for or install an AI** (Hugging Face, Ollama). Release builds include what runs the AI. GitHub is contacted if that piece is missing (typically `pnpm tauri dev` without `pnpm fetch-engine`), and on some Windows machines a faster copy may be downloaded the first time Chat runs.

Those requests send a query string, IP address, and the user agent `Larra/0.9.3 (local-first open-source desktop AI; https://github.com/espilber/larra)`. They do not include Shelf documents.

With Online on in Settings, Chat can also look things up on the public web. Each query or address is shown for approval before it is sent. Approved lookups leave the machine directly and do not go through Larra, and they carry a user agent naming the project and a contact address. Chat is asked not to put private details in them.

A threat model lives in [docs/privacy.md](docs/privacy.md).

## Reporting a vulnerability

**Do not open a public issue** for a vulnerability that could leak local documents or run code in the window.

Email **github.com/espilber/larra/issues**, or open a [private advisory](https://github.com/espilber/larra/security/advisories/new) on GitHub. Either way, include:

- A short description
- Affected version / commit
- Steps to reproduce
- Impact (especially anything that reaches extracted text, conversations, or the network)

We will acknowledge within a few days and work on a fix before any public write-up.

## Scope

In scope: the Larra desktop app, commands that the window can call, AI download and install, Markdown shown in the window, path handling for Shelves and conversations.

Out of scope: third-party AI weights, the upstream runtime, Hugging Face / Ollama availability, physical access to an unlocked machine.
