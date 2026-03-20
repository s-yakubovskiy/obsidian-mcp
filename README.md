# obsidian-mcp

[![CI](https://github.com/lstpsche/obsidian-mcp/actions/workflows/ci.yml/badge.svg)](https://github.com/lstpsche/obsidian-mcp/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/obsidian-mcp.svg)](https://crates.io/crates/obsidian-mcp)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A Rust MCP (Model Context Protocol) server that gives AI agents full read/write access to an [Obsidian](https://obsidian.md) vault through direct filesystem operations. No plugins, no REST API, no Obsidian running — just a single static binary.

## Features

- **29 tools** for vault interaction — navigation, note CRUD, search, graph analysis, periodic notes, and more
- **BM25 ranked search** — Tantivy-powered full-text index with English stemming, field boosts, fuzzy matching, and per-field filtering
- **Semantic search** — local embedding model (fastembed/ONNX) with cosine similarity; hybrid BM25 + semantic re-ranking for best-of-both results
- **Regex search** across all notes with context snippets
- **Wikilink graph** — backlinks, outgoing links, broken link detection, orphan discovery
- **Frontmatter operations** — get, set, remove fields; query notes by metadata
- **Patch operations** — surgically edit heading sections, block references, or frontmatter without rewriting the entire note
- **Periodic notes** — daily, weekly, monthly, quarterly, yearly note support with Obsidian-compatible date formats
- **Live index** with filesystem watcher — all indices (metadata, BM25, embeddings) stay current as files change
- **Path-traversal protection** — all operations validated to stay inside the vault root
- **Cross-platform** — Linux, macOS, Windows

## Installation

### From crates.io

```sh
cargo install obsidian-mcp
```

To enable **semantic (embedding) search**, install with the `embeddings` feature:

```sh
cargo install obsidian-mcp --features embeddings
```

This adds ~60 MB to the binary (ONNX runtime + default model). The model (`BAAI/bge-small-en-v1.5`, 384 dimensions) is downloaded automatically on first run and cached in `.fastembed_cache/`.

### Pre-built binaries

Download from [GitHub Releases](https://github.com/lstpsche/obsidian-mcp/releases/latest):

| Platform | Archive |
|----------|---------|
| Linux x86_64 | `obsidian-mcp-{version}-x86_64-unknown-linux-gnu.tar.gz` |
| Linux ARM64 | `obsidian-mcp-{version}-aarch64-unknown-linux-gnu.tar.gz` |
| macOS Intel | `obsidian-mcp-{version}-x86_64-apple-darwin.tar.gz` |
| macOS Apple Silicon | `obsidian-mcp-{version}-aarch64-apple-darwin.tar.gz` |
| Windows x86_64 | `obsidian-mcp-{version}-x86_64-pc-windows-msvc.zip` |

Extract and place the binary somewhere in your `$PATH`.

## Configuration

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `OBSIDIAN_VAULT_PATH` | Yes* | — | Absolute path to your Obsidian vault |
| `OBSIDIAN_WATCH` | No | `true` | Enable filesystem watcher for live index updates |
| `OBSIDIAN_LOG_LEVEL` | No | `info` | Log level (`trace`, `debug`, `info`, `warn`, `error`) |
| `OBSIDIAN_TANTIVY` | No | `true` | Enable Tantivy BM25 full-text index |
| `OBSIDIAN_EMBEDDINGS` | No | `false` | Enable semantic embedding search (requires `embeddings` feature) |
| `OBSIDIAN_EMBEDDINGS_MODEL` | No | `BAAI/bge-small-en-v1.5` | HuggingFace model for embeddings |

\* Can also be passed as the first CLI argument: `obsidian-mcp /path/to/vault`

## MCP Client Setup

### Cursor

Add to `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "obsidian": {
      "command": "obsidian-mcp",
      "args": ["/path/to/your/vault"]
    }
  }
}
```

### Claude Desktop

Add to your `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "obsidian": {
      "command": "obsidian-mcp",
      "env": {
        "OBSIDIAN_VAULT_PATH": "/path/to/your/vault"
      }
    }
  }
}
```

To enable semantic search, add `OBSIDIAN_EMBEDDINGS` (requires the binary to be compiled with `--features embeddings`):

```json
{
  "mcpServers": {
    "obsidian": {
      "command": "obsidian-mcp",
      "env": {
        "OBSIDIAN_VAULT_PATH": "/path/to/your/vault",
        "OBSIDIAN_EMBEDDINGS": "true"
      }
    }
  }
}
```

Config file locations:
- macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`
- Windows: `%APPDATA%\Claude\claude_desktop_config.json`

## Tool Reference

### Navigation

| Tool | Parameters | Description |
|------|-----------|-------------|
| `vault_list` | `path?`, `recursive?`, `glob?` | List files and directories |
| `vault_structure` | `path?`, `max_depth?` | Tree view of vault structure |

### Note CRUD

| Tool | Parameters | Description |
|------|-----------|-------------|
| `note_read` | `path` | Read full note content |
| `note_create` | `path`, `content?`, `frontmatter?` | Create a new note |
| `note_write` | `path`, `content` | Overwrite note content |
| `note_append` | `path`, `content` | Append to end of note |
| `note_prepend` | `path`, `content` | Insert after frontmatter |
| `note_patch` | `path`, `operation`, `target_type`, `target`, `content` | Patch a heading, block, or frontmatter section |
| `note_delete` | `path`, `confirm` | Delete a note (requires `confirm: true`) |
| `note_move` | `from`, `to` | Move or rename a note |

### Search

| Tool | Parameters | Description |
|------|-----------|-------------|
| `search_text` | `query`, `context_length?`, `max_results?`, `fuzzy?`, `fields?` | BM25 ranked full-text search with stemming |
| `search_semantic` | `query`, `top_k?`, `include_content?`, `lexical_prefetch?` | Semantic similarity search (requires `embeddings` feature) |
| `search_regex` | `pattern`, `context_length?`, `max_results?` | Regex search |
| `search_tag` | `tag`, `include_nested?` | Find notes by tag |
| `search_frontmatter` | `field`, `value?`, `operator` | Query by frontmatter (`eq`, `contains`, `exists`) |

### Metadata

| Tool | Parameters | Description |
|------|-----------|-------------|
| `note_metadata` | `path` | Rich metadata: tags, headings, links, stats |
| `note_document_map` | `path` | List patch targets (headings, blocks, fields) |
| `frontmatter_get` | `path` | Get frontmatter as JSON |
| `frontmatter_set` | `path`, `key`, `value` | Set a frontmatter field |
| `frontmatter_remove` | `path`, `key` | Remove a frontmatter field |

### Graph / Links

| Tool | Parameters | Description |
|------|-----------|-------------|
| `links_backlinks` | `path` | Notes linking TO this note |
| `links_outgoing` | `path` | Outgoing wikilinks with resolution status |
| `links_broken` | `path?` | Broken wikilinks (vault-wide or single note) |
| `links_orphans` | — | Notes with no links in or out |

### Periodic Notes

| Tool | Parameters | Description |
|------|-----------|-------------|
| `periodic_get` | `period`, `date?` | Read a periodic note |
| `periodic_create` | `period`, `date?`, `content?` | Create a periodic note |
| `periodic_list_recent` | `period`, `limit?` | List recent periodic notes |

### Utility

| Tool | Parameters | Description |
|------|-----------|-------------|
| `vault_info` | — | Aggregate vault statistics |
| `open_in_obsidian` | `path`, `new_leaf?` | Open note via `obsidian://` URI |

## Search

obsidian-mcp provides three tiers of search, each suited to different use cases.

### Tier 1: BM25 Full-Text Search (Tantivy)

Enabled by default (`OBSIDIAN_TANTIVY=true`). Uses an in-memory [Tantivy](https://github.com/quickwit-oss/tantivy) inverted index rebuilt on startup (< 100 ms for 5000 notes).

**Capabilities:**
- English stemming — "programming" matches "program", "programs"
- Field-level boosts — title (5x), tags (4x), headings (3x), frontmatter (2x), body (1x)
- Fuzzy matching — `fuzzy: true` tolerates single-character typos (edit distance 1)
- Field filtering — `fields: ["title", "tags"]` restricts search to specific fields (`title`, `headings`, `tags`, `body`, `frontmatter`)
- Results ranked by BM25 score with context snippets around matches

When Tantivy is disabled, `search_text` falls back to a brute-force regex scan.

### Tier 2: Semantic Search (Embeddings)

Opt-in. Requires the `embeddings` Cargo feature **and** `OBSIDIAN_EMBEDDINGS=true` at runtime.

**Prerequisites:**

```sh
# Build with embeddings support
cargo install obsidian-mcp --features embeddings

# Enable at runtime
OBSIDIAN_EMBEDDINGS=true obsidian-mcp /path/to/vault
```

**How it works:**
- Each note is embedded into a 384-dimensional vector using a local ONNX model (`BAAI/bge-small-en-v1.5` by default)
- Vectors are cached to disk at `{vault}/.obsidian/obsidian-mcp/embeddings.bin` — subsequent startups are fast
- `search_semantic` computes cosine similarity between the query embedding and all note embeddings
- No external API calls — everything runs locally

**Hybrid re-ranking:**  Set `lexical_prefetch: true` to combine both tiers. BM25 retrieves the top 50 candidates, then each is re-scored as `0.4 * normalized_bm25 + 0.6 * cosine_similarity`. This gives higher-quality results than either approach alone.

### Tier 3: Regex Search

Always available. `search_regex` does a brute-force scan with a user-provided regex pattern. Useful for precise pattern matching (e.g. finding URLs, specific code patterns, or structured data).

### When to Use What

| Need | Tool |
|------|------|
| General knowledge retrieval | `search_text` |
| Conceptual / "notes about X" | `search_semantic` |
| Best-of-both precision + recall | `search_semantic` with `lexical_prefetch: true` |
| Exact patterns or regex | `search_regex` |
| By tag or metadata | `search_tag` / `search_frontmatter` |

## Architecture

```
AI Client (Cursor / Claude Desktop / etc.)
 │ stdio (MCP JSON-RPC)
 ▼
obsidian-mcp binary
 ├── src/tools/           ← MCP tool handlers
 ├── src/vault/           ← vault layer (fs, index, parser, watcher)
 │   ├── index            ← in-memory metadata index
 │   ├── tantivy_index    ← BM25 full-text search (Tantivy)
 │   ├── embeddings       ← semantic search (fastembed, optional)
 │   └── watcher          ← filesystem watcher, syncs all indices
 ├── src/models.rs        ← shared types
 └── src/error.rs         ← VaultError
 │ filesystem
 ▼
~/vault/ (.md files + .obsidian/ config)
```

The vault layer handles all filesystem operations and maintains up to three indices (metadata, BM25, embeddings). The tools layer translates MCP requests into vault operations. The two layers are cleanly separated — the vault module knows nothing about MCP.

## Development

```sh
# Build
cargo build

# Build with semantic search
cargo build --features embeddings

# Run
OBSIDIAN_VAULT_PATH=~/my-vault cargo run

# Run with all search features
OBSIDIAN_VAULT_PATH=~/my-vault OBSIDIAN_EMBEDDINGS=true cargo run --features embeddings

# Quality checks
cargo fmt --check && cargo clippy && cargo test

# Test with MCP Inspector
npx @modelcontextprotocol/inspector cargo run
```

## License

[MIT](LICENSE)
