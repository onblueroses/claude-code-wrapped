pub const STYLE_BLOCK: &str = r#"<style>
    *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
    body {
      font-family: ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
      background: #000;
      color: #fff;
      -webkit-font-smoothing: antialiased;
      overflow-x: hidden;
    }

    /* ── Slide layout ── */
    .slide {
      width: 100%;
      padding: 80px 40px;
      display: flex;
      align-items: center;
      justify-content: center;
    }
    .slide-inner {
      width: 100%;
      max-width: 860px;
    }
    .stat-slide { padding: 96px 40px; }
    .data-slide { padding: 72px 40px; }

    /* Slide backgrounds */
    .s-black  { background: #000; color: #fff; }
    .s-dark   { background: #121212; color: #fff; }
    .s-green  { background: #1a8a47; color: #fff; }
    .s-coral  { background: #9e2f1a; color: #fff; }
    .s-purple { background: #3d1480; color: #fff; }
    .s-amber  { background: #a06c1a; color: #fff; }

    /* ── Opening slide ── */
    .opening-slide {
      min-height: 100vh;
      align-items: flex-end;
      padding-bottom: 72px;
    }
    .wordmark {
      display: block;
      font-size: 11px;
      font-weight: 700;
      letter-spacing: 0.28em;
      text-transform: uppercase;
      color: #1a8a47;
      margin-bottom: 48px;
    }
    .archetype-title {
      font-size: clamp(56px, 10vw, 120px);
      font-weight: 900;
      line-height: 0.88;
      letter-spacing: -0.04em;
      margin-bottom: 24px;
    }
    .hero-desc {
      font-size: clamp(15px, 2vw, 19px);
      opacity: 0.5;
      line-height: 1.6;
      max-width: 520px;
      margin-bottom: 48px;
    }
    .hero-stats {
      display: flex;
      gap: 36px;
      flex-wrap: wrap;
    }
    .hero-stat-val {
      font-size: clamp(22px, 3vw, 32px);
      font-weight: 800;
      letter-spacing: -0.04em;
      line-height: 1;
    }
    .hero-stat-lbl {
      font-size: 10px;
      font-weight: 700;
      letter-spacing: 0.18em;
      text-transform: uppercase;
      opacity: 0.35;
      margin-top: 6px;
    }

    /* ── Slide typography ── */
    .slide-label {
      font-size: clamp(11px, 1.4vw, 13px);
      font-weight: 700;
      letter-spacing: 0.22em;
      text-transform: uppercase;
      opacity: 0.5;
      margin-bottom: 20px;
    }
    .slide-hero {
      font-size: clamp(72px, 12vw, 148px);
      font-weight: 900;
      line-height: 0.87;
      letter-spacing: -0.04em;
      margin-bottom: 20px;
    }
    .slide-hero-med {
      font-size: clamp(52px, 8vw, 104px);
      font-weight: 900;
      line-height: 0.88;
      letter-spacing: -0.04em;
      margin-bottom: 20px;
    }
    .slide-sub {
      font-size: clamp(15px, 2.2vw, 20px);
      font-weight: 500;
      opacity: 0.6;
      line-height: 1.5;
      max-width: 480px;
    }

    /* ── Cache grade hero ── */
    .cache-grade-hero {
      font-size: clamp(160px, 26vw, 300px);
      font-weight: 900;
      line-height: 0.82;
      letter-spacing: -0.06em;
      margin-bottom: 32px;
    }
    .cache-meta {
      display: flex;
      gap: 36px;
      flex-wrap: wrap;
    }
    .cache-stat-val {
      font-size: clamp(24px, 3vw, 36px);
      font-weight: 800;
      letter-spacing: -0.03em;
      line-height: 1;
    }
    .cache-stat-lbl {
      font-size: 10px;
      font-weight: 700;
      letter-spacing: 0.18em;
      text-transform: uppercase;
      opacity: 0.4;
      margin-top: 6px;
    }

    /* ── Section headers (data slides) ── */
    .section-label {
      font-size: 11px;
      font-weight: 700;
      letter-spacing: 0.22em;
      text-transform: uppercase;
      opacity: 0.35;
      margin-bottom: 10px;
    }
    .section-title {
      font-size: clamp(26px, 3.5vw, 44px);
      font-weight: 900;
      letter-spacing: -0.03em;
      margin-bottom: 32px;
    }

    /* ── Activity chart ── */
    .activity-chart {
      display: flex;
      align-items: flex-end;
      gap: 4px;
      height: 140px;
    }
    .spark-col {
      flex: 1;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: flex-end;
      height: 100%;
      gap: 6px;
    }
    .spark-bar {
      width: 100%;
      border-radius: 3px 3px 0 0;
      background: #1a8a47;
      min-height: 3px;
    }
    .spark-label { font-size: 9px; opacity: 0.3; letter-spacing: 0.02em; }

    /* ── Model rows ── */
    .model-list { display: flex; flex-direction: column; gap: 18px; }
    .model-row { display: flex; flex-direction: column; gap: 7px; }
    .model-row-top { display: flex; justify-content: space-between; align-items: baseline; }
    .model-row-top strong { font-size: 13px; font-weight: 600; }
    .model-row-top span { font-size: 12px; opacity: 0.4; }
    .bar-track { height: 3px; background: rgba(255,255,255,0.08); border-radius: 2px; overflow: hidden; }
    .bar-fill { height: 100%; background: #1a8a47; border-radius: inherit; }

    /* ── Project rows ── */
    .proj-list { display: flex; flex-direction: column; }
    .proj-row {
      display: grid;
      grid-template-columns: 1fr 72px 72px;
      gap: 12px;
      align-items: center;
      padding: 11px 0;
      border-bottom: 1px solid rgba(255,255,255,0.06);
    }
    .proj-row:last-child { border-bottom: none; }
    .proj-name { font-size: 13px; font-weight: 600; margin-bottom: 6px; }
    .proj-bar-wrap { height: 2px; background: rgba(255,255,255,0.08); border-radius: 1px; overflow: hidden; }
    .proj-bar { height: 100%; background: #1a8a47; border-radius: inherit; }
    .proj-sessions, .proj-tokens { font-size: 11px; opacity: 0.35; text-align: right; font-family: ui-monospace, monospace; }

    /* ── Session rows ── */
    .session-list { display: flex; flex-direction: column; }
    .session-row {
      display: flex;
      align-items: flex-start;
      justify-content: space-between;
      gap: 16px;
      padding: 11px 0;
      border-bottom: 1px solid rgba(255,255,255,0.06);
    }
    .session-row:last-child { border-bottom: none; }
    .session-project { font-size: 13px; font-weight: 600; margin-bottom: 3px; }
    .session-meta { font-size: 11px; opacity: 0.35; font-family: ui-monospace, monospace; }
    .session-prompt { font-size: 11px; opacity: 0.35; margin-top: 3px; max-width: 300px; line-height: 1.5; }
    .token-badge {
      background: rgba(29,185,84,0.1);
      color: #1a8a47;
      border: 1px solid rgba(29,185,84,0.22);
      border-radius: 6px;
      padding: 4px 8px;
      font-size: 11px;
      font-weight: 700;
      white-space: nowrap;
      font-family: ui-monospace, monospace;
      flex-shrink: 0;
    }

    /* ── Cards (highlights + recs) ── */
    .card-grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 12px; }
    .card {
      background: #1a1a1a;
      border: 1px solid rgba(255,255,255,0.07);
      border-radius: 12px;
      padding: 20px;
    }
    .eyebrow {
      font-size: 10px;
      font-weight: 700;
      letter-spacing: 0.18em;
      text-transform: uppercase;
      color: #1a8a47;
      margin-bottom: 8px;
    }
    .card h3 { font-size: 14px; font-weight: 700; margin-bottom: 6px; line-height: 1.4; }
    .card p { font-size: 12px; opacity: 0.45; line-height: 1.65; }

    /* ── Ratio bar ── */
    .ratio-bar {
      height: 7px;
      border-radius: 999px;
      overflow: hidden;
      background: rgba(255,255,255,0.1);
      margin: 14px 0 10px;
      display: flex;
    }
    .ratio-human { height: 100%; background: #1a8a47; }
    .ratio-meta { display: flex; justify-content: space-between; font-size: 12px; opacity: 0.45; }

    /* ── Cache savings ── */
    .savings-row {
      display: flex;
      justify-content: space-between;
      align-items: center;
      padding: 10px 0;
      border-bottom: 1px solid rgba(255,255,255,0.07);
      font-size: 13px;
    }
    .savings-row:last-child { border-bottom: none; }
    .s-pos { color: #1a8a47; font-weight: 700; font-family: ui-monospace, monospace; }
    .s-neg { color: #c04030; font-weight: 700; font-family: ui-monospace, monospace; }
    .s-muted { opacity: 0.4; }

    /* ── Inflection note ── */
    .inflection-note {
      font-size: 12px;
      padding: 10px 14px;
      border-radius: 8px;
      margin-top: 20px;
      line-height: 1.55;
    }
    .inflection-note.warn {
      background: rgba(232,71,42,0.1);
      color: #E8472A;
      border: 1px solid rgba(232,71,42,0.2);
    }
    .inflection-note.good {
      background: rgba(29,185,84,0.08);
      color: #1a8a47;
      border: 1px solid rgba(29,185,84,0.2);
    }

    /* ── 2-column layout ── */
    .data-grid-2 { display: grid; grid-template-columns: repeat(2, 1fr); gap: 60px; }

    /* ── Responsive ── */
    @media (max-width: 700px) {
      .slide { padding: 60px 28px; }
      .data-slide { padding: 56px 28px; }
      .card-grid { grid-template-columns: 1fr; }
      .data-grid-2 { grid-template-columns: 1fr; gap: 48px; }
      .hero-stats { gap: 24px; }
      .cache-grade-hero { font-size: 140px; }
    }
    @media (max-width: 420px) {
      .slide { padding: 48px 20px; }
      .opening-slide { padding-bottom: 56px; }
    }
  </style>"#;
