# Tools Repo Scaffold Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bootstrap a personal tools repo with GitHub Pages hosting, a clean index page, and conventions ready for adding tools.

**Architecture:** Static HTML files in `public/`, deployed to GitHub Pages via GitHub Actions on push to `main`. No build step. Index page and README both maintained manually.

**Tech Stack:** Vanilla HTML/CSS, GitHub Actions, GitHub Pages.

---

### Task 1: Initialize git repo and base files

**Files:**
- Create: `.gitignore`
- Create: `docs/superpowers/specs/2026-04-21-tools-repo-design.md` *(already exists — no action needed)*

- [ ] **Step 1: Initialize git repo**

```bash
cd /Users/mainar/dev/personal/tools
git init
git checkout -b main
```

Expected: `Initialized empty Git repository in .../tools/.git/`

- [ ] **Step 2: Create .gitignore**

Create `.gitignore`:

```
.DS_Store
```

- [ ] **Step 3: Commit**

```bash
git add .gitignore docs/
git commit -m "chore: initialize repo with design spec"
```

---

### Task 2: Create GitHub Actions deploy workflow

**Files:**
- Create: `.github/workflows/deploy.yml`

- [ ] **Step 1: Create workflow directory**

```bash
mkdir -p .github/workflows
```

- [ ] **Step 2: Create deploy.yml**

Create `.github/workflows/deploy.yml`:

```yaml
name: Deploy to GitHub Pages

on:
  push:
    branches: [main]

permissions:
  contents: read
  pages: write
  id-token: write

concurrency:
  group: "pages"
  cancel-in-progress: false

jobs:
  deploy:
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/configure-pages@v5
      - uses: actions/upload-pages-artifact@v3
        with:
          path: public/
      - id: deployment
        uses: actions/deploy-pages@v4
```

- [ ] **Step 3: Verify YAML is valid**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/deploy.yml'))" && echo "OK"
```

Expected: `OK`

- [ ] **Step 4: Commit**

```bash
git add .github/
git commit -m "ci: add GitHub Pages deploy workflow"
```

---

### Task 3: Create public/index.html

**Files:**
- Create: `public/index.html`

- [ ] **Step 1: Create public directory**

```bash
mkdir -p public
```

- [ ] **Step 2: Create public/index.html**

Create `public/index.html`:

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Tools</title>
  <style>
    body {
      font-family: system-ui, -apple-system, sans-serif;
      max-width: 800px;
      margin: 0 auto;
      padding: 1rem 2rem;
      color: #333;
    }
    h1 { margin-bottom: 0.25rem; }
    p.subtitle { color: #666; margin-top: 0; }
    ul { list-style: none; padding: 0; }
    li { padding: 0.4rem 0; border-bottom: 1px solid #f0f0f0; }
    li:last-child { border-bottom: none; }
    a { color: #0066cc; text-decoration: none; }
    a:hover { text-decoration: underline; }
    .desc { color: #666; font-size: 0.9em; margin-left: 0.5rem; }
    footer {
      margin-top: 2rem;
      color: #999;
      font-size: 0.85em;
      border-top: 1px solid #eee;
      padding-top: 1rem;
    }
  </style>
</head>
<body>
  <h1>Tools</h1>
  <p class="subtitle">A collection of small browser-based utilities.</p>
  <ul>
    <!-- tools go here, one <li> per tool:
    <li><a href="/tools/tool-name/">Tool Name</a><span class="desc">— one line description</span></li>
    -->
  </ul>
  <footer>
    <a href="https://github.com/ariera/tools">GitHub</a>
  </footer>
</body>
</html>
```

- [ ] **Step 3: Open in browser to verify it looks right**

```bash
open public/index.html
```

Expected: a centered page with "Tools" heading, subtitle, empty list, and GitHub footer link.

- [ ] **Step 4: Commit**

```bash
git add public/
git commit -m "feat: add index page"
```

---

### Task 4: Create README.md

**Files:**
- Create: `README.md`

- [ ] **Step 1: Create README.md**

Create `README.md`:

```markdown
# tools

A collection of small browser-based utilities, hosted at [ariera.github.io/tools](https://ariera.github.io/tools/).

Inspired by [simonw/tools](https://github.com/simonw/tools).

## Tools

<!-- tools go here, one line per tool:
[Tool Name](https://ariera.github.io/tools/tool-name/) — one line description
-->

## Adding a tool

1. Create `public/<tool-name>/index.html` with the tool and a `← All tools` link at the top
2. Add an entry to `public/index.html`
3. Add an entry to the Tools section of this README
4. Commit and push — GitHub Actions deploys automatically
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: add README with conventions"
```

---

### Task 5: Push to GitHub and enable Pages

> This task requires manual steps in the GitHub web UI.

- [ ] **Step 1: Create the repo on GitHub**

Go to https://github.com/new and create a public repo named `tools`. Do not initialize with any files.

- [ ] **Step 2: Add remote and push**

```bash
git remote add origin https://github.com/ariera/tools.git
git push -u origin main
```

Expected: push succeeds, GitHub Actions workflow triggers.

- [ ] **Step 3: Enable GitHub Pages in repo settings**

In the GitHub repo → Settings → Pages:
- Source: **GitHub Actions** (not a branch — the workflow handles deployment)

- [ ] **Step 4: Verify deployment**

Wait ~60 seconds, then check:
- GitHub Actions tab: `Deploy to GitHub Pages` workflow should show green
- Open `https://ariera.github.io/tools/` — should show the index page

- [ ] **Step 5: Verify the deploy workflow URL in the Actions tab**

The workflow job summary should show the deployed URL. Click it to confirm the page loads.
