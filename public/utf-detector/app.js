const inputText = document.getElementById("inputText");
const analyzeBtn = document.getElementById("analyzeBtn");
const clearBtn = document.getElementById("clearBtn");
const renderedText = document.getElementById("renderedText");
const resultBody = document.getElementById("resultBody");
const summaryText = document.getElementById("summaryText");
const activeFilter = document.getElementById("activeFilter");

let detectedMap = new Map();
let activeKey = null;
const FONT_SUPPORT_MATRIX = window.FONT_SUPPORT_MATRIX || {};

const STRANGE_CHAR_OVERRIDES = [
  { cp: 0x0009, name: "TAB", category: "C0_CONTROL", display: "⇥" },
  { cp: 0x000a, name: "LINE FEED (LF)", category: "LINE_BREAK", display: "␊" },
  { cp: 0x000d, name: "CARRIAGE RETURN (CR)", category: "LINE_BREAK", display: "␍" },
  { cp: 0x0085, name: "NEXT LINE (NEL)", category: "LINE_BREAK", display: "␤" },
  { cp: 0x00a0, name: "NO-BREAK SPACE", category: "SPECIAL_WHITESPACE", display: "⍽" },
  { cp: 0x00ad, name: "SOFT HYPHEN", category: "SPECIAL_WHITESPACE", display: "SHY" },
  { cp: 0x034f, name: "COMBINING GRAPHEME JOINER", category: "INVISIBLE_FORMAT", display: "CGJ" },
  { cp: 0x200b, name: "ZERO WIDTH SPACE", category: "ZERO_WIDTH_OR_DIRECTIONAL", display: "ZWSP" },
  { cp: 0x200c, name: "ZERO WIDTH NON-JOINER", category: "ZERO_WIDTH_OR_DIRECTIONAL", display: "ZWNJ" },
  { cp: 0x200d, name: "ZERO WIDTH JOINER", category: "ZERO_WIDTH_OR_DIRECTIONAL", display: "ZWJ" },
  { cp: 0x2028, name: "LINE SEPARATOR", category: "LINE_BREAK", display: "↩" },
  { cp: 0x2029, name: "PARAGRAPH SEPARATOR", category: "LINE_BREAK", display: "¶" },
  { cp: 0x202a, name: "LEFT-TO-RIGHT EMBEDDING", category: "DIRECTIONAL_OVERRIDE", display: "LRE" },
  { cp: 0x202b, name: "RIGHT-TO-LEFT EMBEDDING", category: "DIRECTIONAL_OVERRIDE", display: "RLE" },
  { cp: 0x202c, name: "POP DIRECTIONAL FORMATTING", category: "DIRECTIONAL_OVERRIDE", display: "PDF" },
  { cp: 0x202d, name: "LEFT-TO-RIGHT OVERRIDE", category: "DIRECTIONAL_OVERRIDE", display: "LRO" },
  { cp: 0x202e, name: "RIGHT-TO-LEFT OVERRIDE", category: "DIRECTIONAL_OVERRIDE", display: "RLO" },
  { cp: 0x2060, name: "WORD JOINER", category: "INVISIBLE_FORMAT", display: "WJ" },
  { cp: 0x2066, name: "LEFT-TO-RIGHT ISOLATE", category: "INVISIBLE_FORMAT", display: "LRI" },
  { cp: 0x2067, name: "RIGHT-TO-LEFT ISOLATE", category: "INVISIBLE_FORMAT", display: "RLI" },
  { cp: 0x2068, name: "FIRST STRONG ISOLATE", category: "INVISIBLE_FORMAT", display: "FSI" },
  { cp: 0x2069, name: "POP DIRECTIONAL ISOLATE", category: "INVISIBLE_FORMAT", display: "PDI" },
  { cp: 0x2011, name: "NON-BREAKING HYPHEN", category: "SPECIAL_PUNCTUATION", display: "‑" },
  { cp: 0xfeff, name: "ZERO WIDTH NO-BREAK SPACE (BOM)", category: "SPECIAL_WHITESPACE", display: "BOM" }
];

const STRANGE_CHAR_RANGES = [
  { start: 0x0000, end: 0x001f, category: "C0_CONTROL", nameFor: c0ControlName, displayFor: () => "CTRL" },
  { start: 0x007f, end: 0x009f, category: "C1_CONTROL", nameFor: c1ControlName, displayFor: () => "CTRL" },
  {
    start: 0x200b,
    end: 0x200f,
    category: "ZERO_WIDTH_OR_DIRECTIONAL",
    nameFor: (cp) => `FORMAT CHARACTER (${codepointHex(cp)})`,
    displayFor: () => "INV"
  },
  {
    start: 0x202a,
    end: 0x202e,
    category: "DIRECTIONAL_OVERRIDE",
    nameFor: (cp) => `DIRECTIONAL OVERRIDE (${codepointHex(cp)})`,
    displayFor: () => "DIR"
  },
  {
    start: 0x2060,
    end: 0x206f,
    category: "INVISIBLE_FORMAT",
    nameFor: (cp) => `INVISIBLE FORMAT (${codepointHex(cp)})`,
    displayFor: () => "INV"
  }
];

const STRANGE_CHARACTER_REGISTRY = buildStrangeCharacterRegistry();
const STRANGE_CHARACTER_LIST = [...STRANGE_CHARACTER_REGISTRY.values()].sort((a, b) => a.cp - b.cp);

const classifier = {
  classifyChar(ch) {
    const base = STRANGE_CHARACTER_REGISTRY.get(ch);
    if (!base) return null;
    return { ...base, key: `${base.ch}|${base.code}` };
  },

  shouldFlag(ch) {
    return STRANGE_CHARACTER_REGISTRY.has(ch);
  }
};

function codepointHex(cp) {
  return `U+${cp.toString(16).toUpperCase().padStart(cp <= 0xffff ? 4 : 6, "0")}`;
}

function c0ControlName(cp) {
  const names = [
    "NULL",
    "START OF HEADING",
    "START OF TEXT",
    "END OF TEXT",
    "END OF TRANSMISSION",
    "ENQUIRY",
    "ACKNOWLEDGE",
    "BELL",
    "BACKSPACE",
    "HORIZONTAL TABULATION",
    "LINE FEED",
    "VERTICAL TABULATION",
    "FORM FEED",
    "CARRIAGE RETURN",
    "SHIFT OUT",
    "SHIFT IN",
    "DATA LINK ESCAPE",
    "DEVICE CONTROL ONE",
    "DEVICE CONTROL TWO",
    "DEVICE CONTROL THREE",
    "DEVICE CONTROL FOUR",
    "NEGATIVE ACKNOWLEDGE",
    "SYNCHRONOUS IDLE",
    "END OF TRANSMISSION BLOCK",
    "CANCEL",
    "END OF MEDIUM",
    "SUBSTITUTE",
    "ESCAPE",
    "FILE SEPARATOR",
    "GROUP SEPARATOR",
    "RECORD SEPARATOR",
    "UNIT SEPARATOR"
  ];
  return names[cp] || `C0 CONTROL (${codepointHex(cp)})`;
}

function c1ControlName(cp) {
  const names = {
    0x7f: "DELETE",
    0x80: "PADDING CHARACTER",
    0x81: "HIGH OCTET PRESET",
    0x82: "BREAK PERMITTED HERE",
    0x83: "NO BREAK HERE",
    0x84: "INDEX",
    0x85: "NEXT LINE",
    0x86: "START OF SELECTED AREA",
    0x87: "END OF SELECTED AREA",
    0x88: "CHARACTER TABULATION SET",
    0x89: "CHARACTER TABULATION WITH JUSTIFICATION",
    0x8a: "LINE TABULATION SET",
    0x8b: "PARTIAL LINE FORWARD",
    0x8c: "PARTIAL LINE BACKWARD",
    0x8d: "REVERSE LINE FEED",
    0x8e: "SINGLE SHIFT TWO",
    0x8f: "SINGLE SHIFT THREE",
    0x90: "DEVICE CONTROL STRING",
    0x91: "PRIVATE USE ONE",
    0x92: "PRIVATE USE TWO",
    0x93: "SET TRANSMIT STATE",
    0x94: "CANCEL CHARACTER",
    0x95: "MESSAGE WAITING",
    0x96: "START OF GUARDED AREA",
    0x97: "END OF GUARDED AREA",
    0x98: "START OF STRING",
    0x99: "SINGLE GRAPHIC CHARACTER INTRODUCER",
    0x9a: "SINGLE CHARACTER INTRODUCER",
    0x9b: "CONTROL SEQUENCE INTRODUCER",
    0x9c: "STRING TERMINATOR",
    0x9d: "OPERATING SYSTEM COMMAND",
    0x9e: "PRIVACY MESSAGE",
    0x9f: "APPLICATION PROGRAM COMMAND"
  };
  return names[cp] || `C1 CONTROL (${codepointHex(cp)})`;
}

function buildStrangeCharacterRegistry() {
  const registry = new Map();

  for (const range of STRANGE_CHAR_RANGES) {
    for (let cp = range.start; cp <= range.end; cp += 1) {
      const ch = String.fromCodePoint(cp);
      registry.set(ch, {
        ch,
        cp,
        code: codepointHex(cp),
        name: range.nameFor(cp),
        category: range.category,
        display: range.displayFor(cp),
        support: FONT_SUPPORT_MATRIX[codepointHex(cp)] || null
      });
    }
  }

  for (const item of STRANGE_CHAR_OVERRIDES) {
    const ch = String.fromCodePoint(item.cp);
    registry.set(ch, {
      ch,
      cp: item.cp,
      code: codepointHex(item.cp),
      name: item.name,
      category: item.category,
      display: item.display,
      support: FONT_SUPPORT_MATRIX[codepointHex(item.cp)] || null
    });
  }

  return registry;
}

function collectDetected(text) {
  const map = new Map();
  for (const ch of text) {
    if (!classifier.shouldFlag(ch)) {
      continue;
    }
    const info = classifier.classifyChar(ch);
    if (!map.has(info.key)) {
      map.set(info.key, { ...info, count: 0 });
    }
    map.get(info.key).count += 1;
  }
  return map;
}

function renderOutput(text) {
  renderedText.replaceChildren();
  if (!text) {
    const empty = document.createElement("div");
    empty.className = "empty-state";
    empty.textContent = "Nothing to render yet.";
    renderedText.appendChild(empty);
    return;
  }

  const fragment = document.createDocumentFragment();

  for (const ch of text) {
    if (classifier.shouldFlag(ch)) {
      const info = classifier.classifyChar(ch);
      const span = document.createElement("span");
      span.className = "flagged";
      span.dataset.charKey = info.key;
      span.title = `${info.code} - ${info.name}`;
      span.textContent = info.display;
      fragment.appendChild(span);
    } else {
      fragment.appendChild(document.createTextNode(ch));
    }
  }

  renderedText.appendChild(fragment);
  applyFilterHighlight();
}

function renderTable() {
  resultBody.replaceChildren();
  const rows = [...detectedMap.values()].sort((a, b) => {
    if (b.count !== a.count) return b.count - a.count;
    return a.cp - b.cp;
  });

  if (rows.length === 0) {
    summaryText.textContent = "No flagged UTF characters found.";
    const tr = document.createElement("tr");
    const td = document.createElement("td");
    td.colSpan = 7;
    td.className = "empty-state";
    td.textContent = "No suspicious/non-printable characters detected.";
    tr.appendChild(td);
    resultBody.appendChild(tr);
    activeFilter.textContent = "Showing: all flagged characters";
    return;
  }

  const total = rows.reduce((sum, row) => sum + row.count, 0);
  summaryText.textContent = `${rows.length} unique flagged character(s), ${total} total occurrence(s). Click a row to filter.`;

  for (const row of rows) {
    const tr = document.createElement("tr");
    tr.dataset.charKey = row.key;
    if (row.key === activeKey) tr.classList.add("selected");

    const charTd = document.createElement("td");
    charTd.className = "char-cell";
    const badge = document.createElement("span");
    badge.className = "tag";
    badge.textContent = row.display;
    charTd.appendChild(badge);

    const codeTd = document.createElement("td");
    codeTd.textContent = row.code;

    const decimalTd = document.createElement("td");
    decimalTd.textContent = String(row.cp);

    const tnrSupportTd = document.createElement("td");
    tnrSupportTd.textContent = supportLabel(row.support, "timesNewRoman");

    const helveticaSupportTd = document.createElement("td");
    helveticaSupportTd.textContent = supportLabel(row.support, "helvetica");

    const nameTd = document.createElement("td");
    nameTd.textContent = row.name;

    const countTd = document.createElement("td");
    countTd.textContent = String(row.count);

    tr.appendChild(charTd);
    tr.appendChild(codeTd);
    tr.appendChild(decimalTd);
    tr.appendChild(tnrSupportTd);
    tr.appendChild(helveticaSupportTd);
    tr.appendChild(nameTd);
    tr.appendChild(countTd);

    tr.addEventListener("click", () => {
      if (activeKey === row.key) {
        activeKey = null;
      } else {
        activeKey = row.key;
      }
      renderTable();
      applyFilterHighlight();
    });

    resultBody.appendChild(tr);
  }

  if (!activeKey) {
    activeFilter.textContent = "Showing: all flagged characters";
  } else {
    const info = detectedMap.get(activeKey);
    activeFilter.textContent = `Showing: ${info.display} (${info.code})`;
  }
}

function supportLabel(support, key) {
  if (!support || typeof support[key] !== "boolean") {
    return "N/A";
  }
  return support[key] ? "Yes" : "No";
}

function applyFilterHighlight() {
  const flaggedNodes = renderedText.querySelectorAll(".flagged");
  flaggedNodes.forEach((node) => {
    node.classList.remove("active", "reduced");

    if (!activeKey) {
      node.classList.add("active");
      return;
    }

    if (node.dataset.charKey === activeKey) {
      node.classList.add("active");
    } else {
      node.classList.add("reduced");
    }
  });
}

function analyze() {
  const text = inputText.value;
  detectedMap = collectDetected(text);

  if (activeKey && !detectedMap.has(activeKey)) {
    activeKey = null;
  }

  renderOutput(text);
  renderTable();
}

analyzeBtn.addEventListener("click", analyze);
clearBtn.addEventListener("click", () => {
  inputText.value = "";
  detectedMap = new Map();
  activeKey = null;
  renderedText.replaceChildren();
  resultBody.replaceChildren();
  summaryText.textContent = "No analysis yet.";
  activeFilter.textContent = "Showing: all flagged characters";
});

inputText.addEventListener("keydown", (event) => {
  if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
    analyze();
  }
});

const sample = [
  "Windows line ending\\r\\n demo:\r\nLine B",
  "Classic Mac line ending demo:\rLegacy line",
  "Unix line ending demo:\nLine B",
  "Hidden marks: zero-width space->\u200B<- and NBSP->\u00A0<- and BOM->\uFEFF<-",
  "Non-breaking hyphen (U+2011): word‑join"
].join("\n\n");

inputText.value = sample;
analyze();
