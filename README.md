# tools

A collection of small browser-based utilities, hosted at [ariera.github.io/tools](https://ariera.github.io/tools/).

Inspired by [simonw/tools](https://github.com/simonw/tools).

## Tools

[SVG → macOS Icon](https://ariera.github.io/tools/svg-to-icns/) — convert an SVG into a .icns file with all standard sizes
[UTF Character Inspector](https://ariera.github.io/tools/utf-detector/) — detect invisible and unusual Unicode characters in pasted text

## Adding a tool

1. Create `public/<tool-name>/index.html` with the tool and a `← All tools` link at the top
2. Add an entry to `public/index.html`
3. Add an entry to the Tools section of this README
4. Commit and push — GitHub Actions deploys automatically
