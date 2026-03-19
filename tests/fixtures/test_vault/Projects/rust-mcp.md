---
tags: [rust, mcp, project]
aliases: [obsidian-mcp]
status: active
---
# Rust MCP Server

A Model Context Protocol server written in Rust. ^intro

## Architecture

The server uses direct filesystem access instead of HTTP APIs.
See [[getting-started]] for setup instructions.

## Features

- Full-text search across notes
- Wikilink resolution with [[python-tools|Python companion]]
- Tag-based filtering #rust #backend

## Implementation Notes ^impl

Uses `rmcp` crate for MCP protocol handling. ^dep-note
