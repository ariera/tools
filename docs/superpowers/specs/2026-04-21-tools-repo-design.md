# Tools Repo Design

**Date:** 2026-04-21
**Status:** Approved

## Goal

A personal tools repository hosted on GitHub Pages at `ariera.github.io/tools/`, inspired by simonw/tools. Houses small, self-contained browser-based utilities. Static only — no backend, no build step.

## Repository Structure

```
tools/                          ← git repo root
├── .github/
│   └── workflows/
│       └── deploy.yml          ← deploys public/ to GitHub Pages
├── public/
│   └── index.html              ← tool listing page
├── .gitignore
└── README.md                   ← human-readable index with GitHub Pages links
```

Each tool added later:

```
public/
└── <tool-name>/
    └── index.html              ← self-contained tool page
```

Multi-file tools (tools that need extra assets) also live under `public/<tool-name>/`.

## URLs

- Index: `https://ariera.github.io/tools/`
- Tools: `https://ariera.github.io/tools/<tool-name>/`

GitHub Pages serves from the `public/` folder via GitHub Actions deploy. The repo is named `tools`, so the base path is `/tools/`.

## GitHub Pages Deployment

- Trigger: push to `main`
- Action: `actions/upload-pages-artifact` + `actions/deploy-pages`
- Source: `public/` directory
- Permissions: `pages: write`, `id-token: write`

## Style Conventions

All tools follow these conventions, matching simonw's aesthetic:

- **Fonts:** `system-ui, -apple-system, sans-serif` — no external font loads
- **Layout:** centered container, `max-width: 800px`, `margin: 0 auto`, `padding: 1rem`
- **Colors:** `#333` for body text, `#0066cc` for links/interactive elements, `#f5f5f5` for subtle backgrounds
- **No frameworks:** vanilla HTML/CSS/JS only; external libraries only when a specific tool genuinely needs one
- **Navigation:** each tool page has a `← All tools` link at the top pointing to `/tools/`
- **Self-contained:** each tool's `index.html` includes all its CSS and JS inline

## Index Page (`public/index.html`)

- Plain `<ul>` list of tools, each item is a linked tool name with a one-line description
- Grouped by category once there are enough tools to warrant it; flat list to start
- Same style conventions as tools (system font, centered, minimal)
- Updated manually when a new tool is added

## README

- Acts as the repo's human-readable index
- Links use the full GitHub Pages URL: `https://ariera.github.io/tools/<tool-name>/`
- One line per tool: `[Tool Name](URL) — brief description`
- Updated alongside `public/index.html` when a tool is added

## Adding a Tool (Convention)

1. Create `public/<tool-name>/index.html`
2. Add `← All tools` link at top of tool page
3. Add entry to `public/index.html`
4. Add entry to `README.md`
5. Commit and push — GitHub Actions deploys automatically

## Out of Scope

- Build scripts / index generators (can be added later when there are 10+ tools)
- Custom domain
- Backend / serverless tools (different hosting needed)
- Analytics
