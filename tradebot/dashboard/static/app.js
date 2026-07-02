(function () {
    "use strict";

    // ---------- formatting helpers ----------
    const fmt = (n, digits = 2) =>
        n == null || Number.isNaN(Number(n)) ? "—" : Number(n).toFixed(digits);

    const fmtMoney = (n, digits = 2) => (n == null ? "—" : "$" + fmt(n, digits));

    const fmtPct = (n) => (n == null ? "—" : (n * 100).toFixed(2) + "%");

    const fmtSignedMoney = (n) => {
        if (n == null) return { html: "—", cls: "" };
        const cls = n > 0 ? "pnl-pos" : n < 0 ? "pnl-neg" : "";
        const sign = n > 0 ? "+" : "";
        return { html: `${sign}$${Math.abs(n).toFixed(4)}`, cls };
    };

    const fmtSignedPct = (n) => {
        if (n == null) return { html: "—", cls: "" };
        const cls = n > 0 ? "pnl-pos" : n < 0 ? "pnl-neg" : "";
        const sign = n > 0 ? "+" : "";
        return { html: `${sign}${(n * 100).toFixed(2)}%`, cls };
    };

    const fmtTime = (iso) => {
        if (!iso) return "—";
        const t = iso.replace("T", " ");
        return t.length >= 19 ? t.slice(0, 19) : t;
    };

    const fmtTimeShort = (iso) => {
        if (!iso) return "—";
        return iso.length >= 19 ? iso.slice(11, 19) : iso;
    };

    // ---------- chart ----------
    let chart = null;
    function initChart() {
        const canvas = document.getElementById("equity-chart");
        if (!canvas) return;
        const ctx = canvas.getContext("2d");
        chart = new Chart(ctx, {
            type: "line",
            data: {
                labels: [],
                datasets: [
                    {
                        label: "Equity",
                        data: [],
                        borderColor: "#22c55e",
                        backgroundColor: (context) => {
                            const { chart } = context;
                            const { ctx, chartArea } = chart;
                            if (!chartArea) return "rgba(34,197,94,0.1)";
                            const g = ctx.createLinearGradient(0, chartArea.top, 0, chartArea.bottom);
                            g.addColorStop(0, "rgba(34, 197, 94, 0.22)");
                            g.addColorStop(1, "rgba(34, 197, 94, 0)");
                            return g;
                        },
                        borderWidth: 1.6,
                        tension: 0.25,
                        pointRadius: 0,
                        pointHoverRadius: 4,
                        pointHoverBorderWidth: 2,
                        pointHoverBackgroundColor: "#22c55e",
                        pointHoverBorderColor: "#0a0b10",
                        fill: true,
                    },
                ],
            },
            options: {
                responsive: true,
                maintainAspectRatio: false,
                animation: false,
                interaction: { mode: "index", intersect: false },
                scales: {
                    x: {
                        ticks: {
                            color: "#5b6473",
                            maxTicksLimit: 8,
                            font: { family: "IBM Plex Mono, monospace", size: 10 },
                        },
                        grid: { color: "rgba(255,255,255,0.03)" },
                        border: { color: "#1f2532" },
                    },
                    y: {
                        ticks: {
                            color: "#5b6473",
                            font: { family: "IBM Plex Mono, monospace", size: 10 },
                            callback: (v) => "$" + Number(v).toFixed(2),
                        },
                        grid: { color: "rgba(255,255,255,0.04)" },
                        border: { color: "#1f2532" },
                    },
                },
                plugins: {
                    legend: { display: false },
                    tooltip: {
                        backgroundColor: "#141821",
                        borderColor: "#2a3140",
                        borderWidth: 1,
                        titleColor: "#9aa3b2",
                        titleFont: { family: "IBM Plex Mono, monospace", size: 11 },
                        bodyColor: "#e8eaed",
                        bodyFont: { family: "IBM Plex Mono, monospace", size: 12 },
                        padding: 10,
                        displayColors: false,
                        callbacks: {
                            label: (item) => "Equity: $" + Number(item.parsed.y).toFixed(4),
                        },
                    },
                },
            },
        });
    }

    // ---------- render ----------
    function setText(id, value) {
        const el = document.getElementById(id);
        if (el) el.textContent = value;
    }

    function setHTML(id, html) {
        const el = document.getElementById(id);
        if (el) el.innerHTML = html;
    }

    function renderMode(mode) {
        const pill = document.getElementById("mode-pill");
        if (pill) pill.dataset.mode = mode || "";
        setText("mode", (mode || "—").toUpperCase());
    }

    function renderStats(snap) {
        setText("equity", fmtMoney(snap.equity, 4));
        setText("cash", fmtMoney(snap.cash, 4));

        // SOL gas balance
        const sol = Number(snap.sol_balance) || 0;
        const solSpent = Number(snap.sol_gas_paid_total) || 0;
        const solMark = Number(snap.sol_mark) || 0;
        setText("sol-balance", sol.toFixed(4) + " SOL");
        const sub = document.getElementById("sol-sub");
        if (sub) {
            const usdValue = solMark > 0 ? ` ($${(sol * solMark).toFixed(2)})` : "";
            sub.textContent = `spent ${solSpent.toFixed(6)}${usdValue}`;
            sub.classList.remove("warn", "empty");
            if (sol <= 0) sub.classList.add("empty");
            else if (sol < 0.005) sub.classList.add("warn");
        }

        const realized = fmtSignedMoney(snap.realized_pnl_total);
        setHTML("realized", `<span class="${realized.cls}">${realized.html}</span>`);

        setText("drawdown", fmtPct(snap.drawdown_pct));
        setText("equity-high", "peak " + fmtMoney(snap.equity_high, 4));

        // equity vs high delta
        if (snap.equity != null && snap.equity_high != null && snap.equity_high > 0) {
            const delta = (snap.equity - snap.equity_high) / snap.equity_high;
            const d = fmtSignedPct(delta);
            setHTML("equity-delta", `vs high <span class="${d.cls}">${d.html}</span>`);
        } else {
            setText("equity-delta", "vs high —");
        }

        const ks = document.getElementById("kill-switch");
        const reason = document.getElementById("kill-reason");
        if (snap.kill_switch_active) {
            ks.textContent = "KILL";
            ks.classList.add("kill");
            if (reason) reason.textContent = snap.kill_switch_reason || "trading halted";
        } else {
            ks.textContent = "OK";
            ks.classList.remove("kill");
            if (reason) reason.textContent = "all systems nominal";
        }
    }

    // ---------- positions + lineage ----------
    const expandedPairs = new Set();

    function badgeMeta(badge) {
        // Returns { cls, label, glyph } for any badge string.
        // Backend may send raw strings like "OPEN", "ADD", "TRIM 50%", "CLOSE +$0.12"
        if (!badge) return { cls: "tag-flat", label: "—", glyph: "·" };
        const up = String(badge).toUpperCase();
        if (up.startsWith("OPEN"))      return { cls: "tag-open",      label: up, glyph: "▲" };
        if (up.startsWith("ADD"))       return { cls: "tag-add",       label: up, glyph: "+" };
        if (up.startsWith("TRIM"))      return { cls: "tag-trim",      label: up, glyph: "◐" };
        if (up.startsWith("CLOSE")) {
            // CLOSE +xxx vs CLOSE -xxx
            if (up.includes("-") && !up.includes("+")) return { cls: "tag-close-neg", label: up, glyph: "▼" };
            return { cls: "tag-close-pos", label: up, glyph: "▼" };
        }
        return { cls: "tag-flat", label: up, glyph: "·" };
    }

    function legClass(leg) {
        const b = (leg.badge || "").toUpperCase();
        if (b.startsWith("OPEN")) return "leg-buy";
        if (b.startsWith("ADD"))  return "leg-add";
        if (b.startsWith("TRIM")) return "leg-trim";
        if (b.startsWith("CLOSE")) {
            return (Number(leg.realized_pnl) || 0) < 0 ? "leg-close-neg" : "leg-close-pos";
        }
        // fallback by side
        return leg.side === "buy" ? "leg-buy" : "leg-trim";
    }

    function renderLineage(pair, lineage) {
        if (!lineage || lineage.length === 0) {
            return `<div class="lineage">
                <div class="lineage-head">
                    <span class="lineage-title">${pair} · trade history</span>
                    <span class="lineage-summary">no lineage data — backend will populate after next trade</span>
                </div>
            </div>`;
        }

        let totalRealized = 0;
        let buyCount = 0, sellCount = 0;
        for (const leg of lineage) {
            if (leg.realized_pnl != null) totalRealized += Number(leg.realized_pnl) || 0;
            if (leg.side === "buy") buyCount++; else sellCount++;
        }
        const totCls = totalRealized > 0 ? "pos" : totalRealized < 0 ? "neg" : "";
        const totSign = totalRealized > 0 ? "+" : totalRealized < 0 ? "−" : "";
        const summary = `${lineage.length} legs · ${buyCount}B / ${sellCount}S · realized <span class="${totCls}">${totSign}$${Math.abs(totalRealized).toFixed(4)}</span>`;

        const legs = lineage
            .map((leg) => {
                const meta = badgeMeta(leg.badge);
                const cls = legClass(leg);
                const ts = (leg.ts || "").slice(0, 19).replace("T", " ");
                const realizedHtml = (() => {
                    if (leg.realized_pnl == null) return `<span class="dim">—</span>`;
                    const v = Number(leg.realized_pnl);
                    const c = v > 0 ? "pos" : v < 0 ? "neg" : "dim";
                    const s = v > 0 ? "+" : v < 0 ? "−" : "";
                    return `<span class="${c}">${s}$${Math.abs(v).toFixed(4)}</span>`;
                })();
                return `<div class="lineage-leg ${cls}">
                    <span class="lineage-time">${ts}</span>
                    <span class="tag-badge ${meta.cls}"><span class="tag-glyph">${meta.glyph}</span>${meta.label}</span>
                    <span class="lineage-num">${fmt(leg.base, 6)}</span>
                    <span class="lineage-num">$${fmt(leg.quote, 4)}</span>
                    <span class="lineage-num dim">@ ${fmt(leg.price, 4)}</span>
                    <span class="lineage-realized">${realizedHtml}</span>
                </div>`;
            })
            .join("");

        return `<div class="lineage">
            <div class="lineage-head">
                <span class="lineage-title">${pair} · trade history</span>
                <span class="lineage-summary">${summary}</span>
            </div>
            <div class="lineage-rail">${legs}</div>
        </div>`;
    }

    function renderPositions(positions) {
        const body = document.querySelector("#positions tbody");
        const empty = document.getElementById("positions-empty");
        const count = document.getElementById("positions-count");
        if (count) count.textContent = String(positions.length);
        if (positions.length === 0) {
            body.innerHTML = "";
            if (empty) empty.style.display = "";
            return;
        }
        if (empty) empty.style.display = "none";

        // Drop expanded state for pairs no longer open
        const openPairs = new Set(positions.map((p) => p.pair));
        for (const pair of [...expandedPairs]) {
            if (!openPairs.has(pair)) expandedPairs.delete(pair);
        }

        const rows = [];
        for (const p of positions) {
            const pnl = fmtSignedMoney(p.unrealized_pnl_quote);
            const pct = fmtSignedPct(p.unrealized_pnl_pct);
            const has = Array.isArray(p.lineage) && p.lineage.length > 0;
            const expanded = expandedPairs.has(p.pair);
            const rowCls = `position-row ${has ? "has-lineage" : "no-lineage"} ${expanded ? "expanded" : ""}`;
            rows.push(`<tr class="${rowCls}" data-pair="${p.pair}">
                <td class="col-chev"><span class="chev">▶</span></td>
                <td class="col-pair">${p.pair}</td>
                <td class="num">${fmt(p.base_amount, 6)}</td>
                <td class="num">${fmt(p.avg_entry_price, 4)}</td>
                <td class="num">${fmt(p.mark_price, 4)}</td>
                <td class="num ${pnl.cls}">${pnl.html}</td>
                <td class="num ${pct.cls}">${pct.html}</td>
                <td class="col-action"><button class="sell-now-btn" data-sell-pair="${p.pair}" title="Sell this position now">▼ Sell</button></td>
            </tr>`);
            rows.push(`<tr class="lineage-row ${expanded ? "" : "collapsed"}" data-lineage-for="${p.pair}">
                <td colspan="8">${expanded ? renderLineage(p.pair, p.lineage || []) : ""}</td>
            </tr>`);
        }
        body.innerHTML = rows.join("");

        // Sell-now button handlers (must be bound BEFORE row click so we can stop propagation).
        body.querySelectorAll(".sell-now-btn").forEach((btn) => {
            btn.addEventListener("click", (e) => {
                e.stopPropagation();
                const pair = btn.dataset.sellPair;
                const pos = positions.find((q) => q.pair === pair);
                if (pos) openSellModal(pos);
            });
        });

        // Row expand/collapse
        body.querySelectorAll(".position-row").forEach((tr) => {
            tr.addEventListener("click", () => {
                const pair = tr.dataset.pair;
                if (expandedPairs.has(pair)) {
                    expandedPairs.delete(pair);
                } else {
                    expandedPairs.add(pair);
                }
                const pos = positions.find((q) => q.pair === pair);
                if (!pos) return;
                tr.classList.toggle("expanded", expandedPairs.has(pair));
                const lr = body.querySelector(`tr.lineage-row[data-lineage-for="${pair}"]`);
                if (lr) {
                    if (expandedPairs.has(pair)) {
                        lr.classList.remove("collapsed");
                        lr.querySelector("td").innerHTML = renderLineage(pair, pos.lineage || []);
                    } else {
                        lr.classList.add("collapsed");
                        lr.querySelector("td").innerHTML = "";
                    }
                }
            });
        });
    }

    // ---------- manual sell modal ----------
    let sellTarget = null; // current { pair, base, entry, mark, sol_mark }

    function openSellModal(pos) {
        const snap = lastSnap || {};
        const sol_mark = Number(snap.sol_mark) || 0;
        sellTarget = {
            pair: pos.pair,
            base: Number(pos.base_amount) || 0,
            entry: Number(pos.avg_entry_price) || 0,
            mark: Number(pos.mark_price) || 0,
            sol_mark: sol_mark,
        };
        // Estimate gas as base 5000 lamports + priority fee × 200k CU / 1M.
        // We only have the priority fee in config; pull it from the snapshot if exposed,
        // otherwise fall back to the base fee alone (a conservative under-estimate).
        const priorityFee = (snap.config && Number(snap.config.priority_fee_microlamports)) || 0;
        const lamports = 5000 + (priorityFee * 200000) / 1_000_000;
        const sol_gas = lamports / 1e9;

        const proceeds = sellTarget.base * sellTarget.mark;
        const cost = sellTarget.base * sellTarget.entry;
        const pnl = proceeds - cost;
        const pnlPct = cost > 0 ? pnl / cost : 0;
        const gasUsd = sol_gas * sol_mark;
        const net = pnl - gasUsd;

        const set = (id, v) => { const el = document.getElementById(id); if (el) el.textContent = v; };
        const setCls = (id, cls) => {
            const el = document.getElementById(id);
            if (!el) return;
            el.classList.remove("pos", "neg");
            if (cls) el.classList.add(cls);
        };
        const cls = (n) => (n > 0 ? "pos" : n < 0 ? "neg" : "");
        const sign = (n) => (n > 0 ? "+" : n < 0 ? "−" : "");

        set("sell-modal-pair", sellTarget.pair);
        set("sell-modal-size", `${sellTarget.base.toFixed(6)} ${sellTarget.pair.split("/")[0]}`);
        set("sell-modal-entry", `$${sellTarget.entry.toFixed(4)}`);
        set("sell-modal-mark", `$${sellTarget.mark.toFixed(4)}`);
        set("sell-modal-proceeds", `$${proceeds.toFixed(4)}`);
        set("sell-modal-pnl", `${sign(pnl)}$${Math.abs(pnl).toFixed(4)}`);
        setCls("sell-modal-pnl", cls(pnl));
        set("sell-modal-pnl-pct", `${sign(pnlPct)}${(Math.abs(pnlPct) * 100).toFixed(2)}%`);
        setCls("sell-modal-pnl-pct", cls(pnlPct));
        set("sell-modal-gas", `${sol_gas.toFixed(6)} SOL${sol_mark > 0 ? ` (≈ $${gasUsd.toFixed(4)})` : ""}`);
        set("sell-modal-net", `${sign(net)}$${Math.abs(net).toFixed(4)}`);
        setCls("sell-modal-net", cls(net));
        set("sell-modal-status", "");
        const reasonEl = document.getElementById("sell-modal-reason");
        if (reasonEl) reasonEl.value = "";
        const confirmBtn = document.getElementById("sell-modal-confirm");
        if (confirmBtn) confirmBtn.disabled = false;

        const modal = document.getElementById("sell-modal");
        if (modal) modal.hidden = false;
        if (reasonEl) setTimeout(() => reasonEl.focus(), 50);
    }

    function closeSellModal() {
        const modal = document.getElementById("sell-modal");
        if (modal) modal.hidden = true;
        sellTarget = null;
    }

    async function confirmSell() {
        if (!sellTarget) return;
        const reasonEl = document.getElementById("sell-modal-reason");
        const status = document.getElementById("sell-modal-status");
        const confirmBtn = document.getElementById("sell-modal-confirm");
        const reason = reasonEl ? reasonEl.value.trim() : "";
        if (status) { status.textContent = "submitting…"; status.className = "sell-modal-status mono"; }
        if (confirmBtn) confirmBtn.disabled = true;
        try {
            const url = `/api/positions/${encodeURIComponent(sellTarget.pair)}/sell`;
            const r = await fetch(url, {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ reason: reason }),
            });
            const body = await r.json().catch(() => ({}));
            if (!r.ok) {
                if (status) {
                    status.textContent = `error: ${body.error || ("HTTP " + r.status)}`;
                    status.className = "sell-modal-status mono err";
                }
                if (confirmBtn) confirmBtn.disabled = false;
                return;
            }
            if (status) {
                status.textContent = "queued — executes within one cycle";
                status.className = "sell-modal-status mono ok";
            }
            setTimeout(closeSellModal, 900);
        } catch (e) {
            if (status) {
                status.textContent = `request failed: ${e.message}`;
                status.className = "sell-modal-status mono err";
            }
            if (confirmBtn) confirmBtn.disabled = false;
        }
    }

    function setupSellModal() {
        const modal = document.getElementById("sell-modal");
        if (!modal) return;
        modal.querySelectorAll('[data-action="cancel"]').forEach((el) => {
            el.addEventListener("click", closeSellModal);
        });
        const confirmBtn = document.getElementById("sell-modal-confirm");
        if (confirmBtn) confirmBtn.addEventListener("click", confirmSell);
        document.addEventListener("keydown", (e) => {
            if (e.key === "Escape" && !modal.hidden) closeSellModal();
            if (e.key === "Enter" && !modal.hidden && document.activeElement?.id === "sell-modal-reason") {
                confirmSell();
            }
        });
    }

    function renderSignals(signals) {
        const list = document.getElementById("signals-list");
        if (!list) return;
        if (!signals || signals.length === 0) {
            list.innerHTML = `<div class="empty">no signals yet</div>`;
            return;
        }
        list.innerHTML = signals
            .map((s) => {
                const v = Math.max(-1, Math.min(1, Number(s.composite) || 0));
                const pos = v >= 0;
                // bar from center: width is |v| * 50%, anchored at 50% to either side
                const width = Math.abs(v) * 50;
                const left = pos ? 50 : 50 - width;
                const cls = pos ? "pos" : "neg";
                const valCls = v > 0 ? "pos" : v < 0 ? "neg" : "";
                const sign = v > 0 ? "+" : "";
                return `<div class="signal-row">
                    <div class="signal-pair">${s.pair}</div>
                    <div class="signal-bar">
                        <div class="signal-fill ${cls}" style="left: ${left}%; width: ${width}%;"></div>
                    </div>
                    <div class="signal-value ${valCls}">${sign}${v.toFixed(2)}</div>
                </div>`;
            })
            .join("");
    }

    function renderTrades(trades) {
        const body = document.querySelector("#trades tbody");
        const empty = document.getElementById("trades-empty");
        const count = document.getElementById("trades-count");
        if (count) count.textContent = String(trades.length);
        if (trades.length === 0) {
            body.innerHTML = "";
            if (empty) empty.style.display = "";
            return;
        }
        if (empty) empty.style.display = "none";
        body.innerHTML = trades
            .slice(0, 25)
            .map((t) => {
                const sideCls = t.side === "buy" ? "side-buy" : "side-sell";
                const meta = badgeMeta(t.badge);
                const tagHtml = `<span class="tag-badge ${meta.cls}"><span class="tag-glyph">${meta.glyph}</span>${meta.label}</span>`;
                let realizedHtml = `<span class="realized-cell dim">—</span>`;
                if (t.realized_pnl != null) {
                    const v = Number(t.realized_pnl);
                    const c = v > 0 ? "pos" : v < 0 ? "neg" : "dim";
                    const s = v > 0 ? "+" : v < 0 ? "−" : "";
                    realizedHtml = `<span class="realized-cell ${c}">${s}$${Math.abs(v).toFixed(4)}</span>`;
                }
                return `<tr>
                    <td class="col-time">${fmtTime(t.opened_at)}</td>
                    <td class="col-pair">${t.pair}</td>
                    <td class="${sideCls}">${(t.side || "").toUpperCase()}</td>
                    <td>${tagHtml}</td>
                    <td class="num">${fmt(t.base_amount, 6)}</td>
                    <td class="num">${fmt(t.quote_amount, 4)}</td>
                    <td class="num">${fmt(t.price, 4)}</td>
                    <td class="num">${realizedHtml}</td>
                </tr>`;
            })
            .join("");
    }

    // ---------- per-pair market sparklines ----------
    const sparkCharts = {}; // pair -> Chart instance

    function ensureSparkCanvas(pair) {
        const id = "spark-" + pair.replace(/[^A-Za-z0-9]/g, "_");
        return id;
    }

    function renderMarkets(pairCharts) {
        const grid = document.getElementById("markets-grid");
        const meta = document.getElementById("markets-meta");
        if (!grid) return;

        if (!pairCharts || pairCharts.length === 0) {
            grid.innerHTML = `<div class="empty">no price data yet</div>`;
            if (meta) meta.textContent = "awaiting first ticks…";
            return;
        }

        // total ticks across all pairs for the header meta
        const totalPts = pairCharts.reduce((sum, p) => sum + (p.points?.length || 0), 0);
        if (meta) meta.textContent = `${pairCharts.length} pairs · ${totalPts} pts`;

        // Build (or update) one tile per pair
        const existingPairs = new Set(Object.keys(sparkCharts));
        const seen = new Set();

        pairCharts.forEach((p) => {
            seen.add(p.pair);
            const canvasId = ensureSparkCanvas(p.pair);
            let tile = grid.querySelector(`[data-pair="${p.pair}"]`);

            if (!tile) {
                tile = document.createElement("div");
                tile.className = "market-tile";
                tile.dataset.pair = p.pair;
                tile.innerHTML = `
                    <div class="market-head">
                        <span class="market-pair">${p.pair}</span>
                        <span class="market-change"></span>
                    </div>
                    <div class="market-price"></div>
                    <div class="market-meta">
                        <span class="market-low"></span>
                        <span class="market-high"></span>
                    </div>
                    <div class="market-spark"><canvas id="${canvasId}"></canvas></div>
                `;
                // remove the empty placeholder if present
                const empty = grid.querySelector(".empty");
                if (empty) empty.remove();
                grid.appendChild(tile);
            }

            // Update header values
            const change = fmtSignedPct(p.change_pct);
            const changeEl = tile.querySelector(".market-change");
            changeEl.innerHTML = change.html;
            changeEl.className = "market-change " + change.cls.replace("pnl-", "");

            tile.querySelector(".market-price").textContent = "$" + fmt(p.last, p.last < 1 ? 6 : 4);
            tile.querySelector(".market-low").textContent = "L " + fmt(p.low, p.low < 1 ? 6 : 4);
            tile.querySelector(".market-high").textContent = "H " + fmt(p.high, p.high < 1 ? 6 : 4);

            // Update or create the sparkline chart
            const labels = (p.points || []).map((_, i) => String(i));
            const data = (p.points || []).map((pt) => pt.p);
            const color = p.change_pct >= 0 ? "#22c55e" : "#ef4444";

            if (sparkCharts[p.pair]) {
                const c = sparkCharts[p.pair];
                c.data.labels = labels;
                c.data.datasets[0].data = data;
                c.data.datasets[0].borderColor = color;
                c.update("none");
            } else {
                const ctx = document.getElementById(canvasId).getContext("2d");
                sparkCharts[p.pair] = new Chart(ctx, {
                    type: "line",
                    data: {
                        labels,
                        datasets: [{
                            data,
                            borderColor: color,
                            borderWidth: 1.4,
                            pointRadius: 0,
                            tension: 0.3,
                            fill: false,
                        }],
                    },
                    options: {
                        responsive: true,
                        maintainAspectRatio: false,
                        animation: false,
                        scales: { x: { display: false }, y: { display: false } },
                        plugins: { legend: { display: false }, tooltip: { enabled: false } },
                        elements: { line: { borderJoinStyle: "round" } },
                    },
                });
            }
        });

        // Remove stale tiles (pair removed from watchlist)
        existingPairs.forEach((pair) => {
            if (!seen.has(pair)) {
                const tile = grid.querySelector(`[data-pair="${pair}"]`);
                if (tile) tile.remove();
                if (sparkCharts[pair]) {
                    sparkCharts[pair].destroy();
                    delete sparkCharts[pair];
                }
            }
        });
    }

    function renderChart(history) {
        if (!chart || !Array.isArray(history)) return;
        const labels = history.map((p) => fmtTimeShort(p.snapshot_at));
        const data = history.map((p) => p.equity);
        chart.data.labels = labels;
        chart.data.datasets[0].data = data;
        chart.update("none");

        const range = document.getElementById("equity-range");
        if (range && history.length > 0) {
            range.textContent = `${history.length} pts · ${fmtTimeShort(history[0].snapshot_at)} → ${fmtTimeShort(history[history.length - 1].snapshot_at)}`;
        }
    }

    function shortMint(m) {
        if (!m) return "—";
        return m.length > 12 ? `${m.slice(0, 4)}…${m.slice(-4)}` : m;
    }
    function shortWallet(w) {
        if (!w) return "—";
        return w.length > 10 ? `${w.slice(0, 4)}…${w.slice(-4)}` : w;
    }

    function renderWhales(swaps) {
        const tbody = document.querySelector("#whales-table tbody");
        const empty = document.getElementById("whales-empty");
        const meta = document.getElementById("whales-meta");
        if (!swaps || swaps.length === 0) {
            if (tbody) tbody.innerHTML = "";
            if (empty) empty.style.display = "";
            if (meta) meta.textContent = "no off-watchlist activity";
            return;
        }
        if (empty) empty.style.display = "none";
        if (meta) meta.textContent = `${swaps.length} swap${swaps.length === 1 ? "" : "s"} in unwatched tokens`;
        if (!tbody) return;
        // crude direction inference: if in_mint is a stable-coin pattern (ends w/ "v"
        // for USDC) we call it a buy; otherwise mark as "swap". We don't truly know
        // without decimals/USD pricing — so just label the leg flow.
        tbody.innerHTML = swaps.slice(0, 30).map((s) => {
            const ts = (s.ts || "").slice(0, 19).replace("T", " ");
            return `<tr>
                <td class="col-time">${ts}</td>
                <td class="whale-wallet" title="${s.wallet}">${shortWallet(s.wallet)}</td>
                <td class="whale-direction-buy">SWAP</td>
                <td class="whale-mint" title="${s.in_mint}">${shortMint(s.in_mint)}</td>
                <td class="whale-mint" title="${s.out_mint}">→ ${shortMint(s.out_mint)}</td>
            </tr>`;
        }).join("");
    }

    function renderDecisions(decisions) {
        const tbody = document.querySelector("#decisions-table tbody");
        const empty = document.getElementById("decisions-empty");
        const meta = document.getElementById("decisions-meta");
        if (!decisions || decisions.length === 0) {
            if (tbody) tbody.innerHTML = "";
            if (empty) empty.style.display = "";
            if (meta) meta.textContent = "no decisions yet";
            return;
        }
        if (empty) empty.style.display = "none";
        if (meta) meta.textContent = `live · ${decisions.length} entries`;
        if (tbody) {
            tbody.innerHTML = decisions.map(d => {
                const ts = (d.timestamp || "").slice(0, 19).replace("T", " ");
                const cls = "decision-" + d.decision;
                const rcls = "regime-" + (d.regime || "neutral");
                return `<tr>
                    <td class="col-time">${ts}</td>
                    <td class="col-pair">${d.pair}</td>
                    <td class="num">${Number(d.composite).toFixed(3)}</td>
                    <td class="num">${Number(d.mark).toFixed(4)}</td>
                    <td class="${rcls}">${d.regime || "—"}</td>
                    <td class="${cls}">${d.decision.toUpperCase()}</td>
                    <td>${d.reason || ""}</td>
                </tr>`;
            }).join("");
        }
    }

    // ---------- pause / resume controls ----------
    // Optimistic local override — clears as soon as the snapshot reflects it.
    const pendingPause = {}; // { buys?: bool, sells?: bool }
    let pauseInflight = false;

    function effectiveControl(snap) {
        const c = (snap && snap.control) || {};
        const buys  = pendingPause.buys  != null ? pendingPause.buys  : !!c.paused_buys;
        const sells = pendingPause.sells != null ? pendingPause.sells : !!c.paused_sells;
        return { paused_buys: buys, paused_sells: sells };
    }

    function setToggle(id, pressed, label) {
        const el = document.getElementById(id);
        if (!el) return;
        el.setAttribute("aria-pressed", pressed ? "true" : "false");
        const state = el.querySelector(".pause-toggle-state");
        if (state) state.textContent = label;
    }

    function renderControls(snap) {
        const c = effectiveControl(snap);
        const all = c.paused_buys && c.paused_sells;
        setToggle("pause-buys",  c.paused_buys,  c.paused_buys  ? "paused" : "live");
        setToggle("pause-sells", c.paused_sells, c.paused_sells ? "paused" : "live");
        setToggle("pause-all",   all,            all            ? "halted" : "live");

        const banner = document.getElementById("pause-banner");
        const msg = document.getElementById("pause-banner-msg");
        if (!banner) return;
        if (!c.paused_buys && !c.paused_sells) {
            banner.hidden = true;
            return;
        }
        banner.hidden = false;
        let text;
        if (all) text = "all trading paused — bot is observing only";
        else if (c.paused_buys) text = "buys paused — exits & sells still active";
        else text = "sells paused — bot can open new positions but won't close";
        if (msg) msg.textContent = text;
    }

    async function postControl(payload) {
        // Optimistic UI: keep local state regardless of backend response.
        // The next snapshot's `control` field is the source of truth — if the
        // backend is wired and disagrees, reconcilePending() will clear pendingPause.
        // 404 (backend not yet implemented) is logged but does not roll back.
        if (pauseInflight) return;
        pauseInflight = true;
        try {
            const r = await fetch("/api/control", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify(payload),
            });
            if (!r.ok) {
                if (r.status === 404) {
                    console.info("control endpoint not yet wired (404) — UI stays optimistic");
                } else {
                    console.warn("control POST rejected", r.status);
                }
            }
        } catch (e) {
            console.warn("control POST network error", e);
        } finally {
            pauseInflight = false;
        }
    }

    function setupControls() {
        const buys  = document.getElementById("pause-buys");
        const sells = document.getElementById("pause-sells");
        const all   = document.getElementById("pause-all");

        const flip = (flag) => {
            // Derive current state from DOM (always reflects last render).
            const buysOn  = document.getElementById("pause-buys").getAttribute("aria-pressed") === "true";
            const sellsOn = document.getElementById("pause-sells").getAttribute("aria-pressed") === "true";
            if (flag === "buys") {
                pendingPause.buys = !buysOn;
                renderControls({ control: { paused_buys: pendingPause.buys, paused_sells: sellsOn } });
                postControl({ paused_buys: pendingPause.buys });
            } else if (flag === "sells") {
                pendingPause.sells = !sellsOn;
                renderControls({ control: { paused_buys: buysOn, paused_sells: pendingPause.sells } });
                postControl({ paused_sells: pendingPause.sells });
            } else if (flag === "all") {
                const next = !(buysOn && sellsOn);
                pendingPause.buys = next;
                pendingPause.sells = next;
                renderControls({ control: { paused_buys: next, paused_sells: next } });
                postControl({ paused_buys: next, paused_sells: next });
            }
        };

        if (buys)  buys.addEventListener("click", () => flip("buys"));
        if (sells) sells.addEventListener("click", () => flip("sells"));
        if (all)   all.addEventListener("click",   () => flip("all"));
    }

    function reconcilePending(snap) {
        const c = (snap && snap.control) || {};
        if (pendingPause.buys  != null && !!c.paused_buys  === pendingPause.buys)  delete pendingPause.buys;
        if (pendingPause.sells != null && !!c.paused_sells === pendingPause.sells) delete pendingPause.sells;
    }

    let lastSnap = null;

    function render(snap) {
        if (!snap || typeof snap !== "object") return;
        lastSnap = snap;
        reconcilePending(snap);
        renderControls(snap);
        renderMode(snap.mode);
        renderStats(snap);
        renderPositions(snap.positions || []);
        renderSignals(snap.signals || []);
        renderTrades(snap.recent_trades || []);
        renderChart(snap.equity_history || []);
        renderMarkets(snap.pair_charts || []);
        renderDecisions(snap.decisions || []);
        renderWhales(snap.whale_activity || []);

        const ts = snap.now ? fmtTime(snap.now) : "—";
        setText("last-update", `last update · ${ts}`);

        const limEl = document.getElementById("limiter-meta");
        if (limEl && snap.limiter) {
            const l = snap.limiter;
            const utilPct = l.rate_limit_rps > 0 ? (l.recent_rps / l.rate_limit_rps * 100) : 0;
            const cls = l.total_429s > 0 ? "limiter-err" : utilPct > 85 ? "limiter-warn" : "";
            const retryNote = l.total_429s > 0 ? ` · 429s: ${l.total_429s}` : "";
            limEl.className = "status-mid mono " + cls;
            limEl.textContent = `jupiter: ${l.recent_rps.toFixed(2)} rps / ${l.rate_limit_rps.toFixed(2)} (${utilPct.toFixed(0)}%)${retryNote}`;
        } else if (limEl) {
            limEl.textContent = "jupiter: idle";
        }
    }

    // ---------- websocket ----------
    function setConnState(state, text) {
        const el = document.getElementById("connection");
        if (!el) return;
        el.dataset.state = state;
        const txt = el.querySelector(".conn-text");
        if (txt) txt.textContent = text;
    }

    let reconnectTimer = null;
    function connect() {
        const proto = location.protocol === "https:" ? "wss:" : "ws:";
        const ws = new WebSocket(`${proto}//${location.host}/ws`);

        setConnState("pending", "connecting");
        ws.onopen = () => setConnState("ok", "live");
        ws.onclose = () => {
            setConnState("down", "reconnecting");
            if (reconnectTimer) clearTimeout(reconnectTimer);
            reconnectTimer = setTimeout(connect, 2000);
        };
        ws.onerror = () => {
            try { ws.close(); } catch (e) { /* noop */ }
        };
        ws.onmessage = (ev) => {
            try {
                render(JSON.parse(ev.data));
            } catch (e) {
                console.error("bad ws message", e);
            }
        };
    }

    // ---------- tabs ----------
    function setupTabs() {
        document.querySelectorAll(".tab").forEach((btn) => {
            btn.addEventListener("click", () => {
                const target = btn.getAttribute("data-tab");
                document.querySelectorAll(".tab").forEach((b) => b.classList.toggle("active", b === btn));
                document.querySelectorAll(".tab-pane").forEach((p) => {
                    p.hidden = p.id !== `tab-${target}`;
                });
                if (target === "settings") loadConfig();
                if (target === "backtest") loadBtHistory();
            });
        });
    }

    // ---------- config editor (schema-driven) ----------

    const SETTINGS_SCHEMA = [
        {
            id: "strategy",
            title: "Strategy",
            description: "When and how often the bot decides to trade.",
            fields: [
                { path: "app.entry_threshold", label: "Entry conviction threshold",
                  help: "How sure the bot needs to be before opening a new trade. The composite signal ranges from -1 (strong sell) to +1 (strong buy). Higher = pickier (fewer but better trades), lower = more aggressive.",
                  kind: "slider", min: 0, max: 1, step: 0.01, unit: "score" },
                { path: "app.exit_flip_threshold", label: "Exit-on-flip threshold",
                  help: "When an open position's signal flips this far negative, close it. Closer to 0 = quick to bail; more negative = let trades ride through wobbles.",
                  kind: "slider", min: -1, max: 0, step: 0.01, unit: "score" },
                { path: "app.decision_interval_s", label: "Decision cycle interval",
                  help: "How often the bot wakes up, checks prices, and decides what to do. Lower = react faster but use more rate-limit budget. 10s is the sweet spot for Solana intraday.",
                  kind: "slider", min: 2, max: 60, step: 1, unit: "s" },
                { path: "app.fast_tick_interval_s", label: "Chart refresh interval",
                  help: "How often the chart updates with fresh Birdeye prices (independent of decisions). The Birdeye free tier is ~1 rps total — with N watched mints, the realistic floor is N seconds per full refresh. Set to 0 to disable; otherwise use ≥ (mints / rps). On paid Starter tier (30 rps) you can go to 1s.",
                  kind: "slider", min: 0, max: 30, step: 1, unit: "s" },
                { path: "app.simulated_confirm_latency_s", label: "Demo confirmation latency",
                  help: "Mimics the delay between getting a quote and the trade actually filling on chain. The bot re-quotes after this delay and uses the new (worse) number — so demo P&L matches what real mode would deliver.",
                  kind: "slider", min: 0, max: 3, step: 0.1, unit: "s" },
            ],
        },
        {
            id: "risk",
            title: "Risk Limits",
            description: "The brakes that keep one bad day from ending the bot's career.",
            fields: [
                { path: "risk.max_concurrent_positions", label: "Max open positions at once",
                  help: "How many trades can be live simultaneously. More = diversified attention; fewer = more focused capital.",
                  kind: "stepper", min: 1, max: 10, step: 1 },
                { path: "risk.per_trade_size_min", label: "Min position size",
                  help: "Smallest fraction of cash any single trade can use.",
                  kind: "pct", min: 0.05, max: 1.0, step: 0.05 },
                { path: "risk.per_trade_size_max", label: "Max position size",
                  help: "Largest fraction of cash any single trade can use. The bot picks somewhere in this range based on conviction times Kelly fraction.",
                  kind: "pct", min: 0.05, max: 1.0, step: 0.05 },
                { path: "risk.max_slippage_pct", label: "Max slippage tolerance",
                  help: "Reject trades where the swap price would move against you by more than this. Tighter = safer but more rejected trades; looser = more fills at worse prices. Memecoins typically need 1.5-2%.",
                  kind: "pct", min: 0.001, max: 0.05, step: 0.001 },
                { path: "risk.max_trades_per_day", label: "Max trades per day",
                  help: "Hard cap on entries per UTC day. Stops the bot from over-trading in chop.",
                  kind: "stepper", min: 1, max: 100, step: 1 },
                { path: "risk.daily_loss_limit_pct", label: "Daily loss circuit breaker",
                  help: "If you lose more than this percent of starting capital in one day, stop trading until UTC midnight.",
                  kind: "pct", min: 0.01, max: 0.5, step: 0.01 },
                { path: "risk.weekly_loss_limit_pct", label: "Weekly loss circuit breaker",
                  help: "Same idea but over a rolling 7-day window. Catches slow bleeds the daily limit misses.",
                  kind: "pct", min: 0.01, max: 0.5, step: 0.01 },
                { path: "risk.per_trade_kill_pct", label: "Per-trade hard stop",
                  help: "Bail out of a single trade immediately when it's down by this much. Last line of defense before a stop-loss order.",
                  kind: "pct", min: 0.005, max: 0.2, step: 0.005 },
                { path: "risk.drawdown_circuit_pct", label: "Total drawdown circuit",
                  help: "If your equity falls this far below its all-time peak, halt all trading. The big red button.",
                  kind: "pct", min: 0.05, max: 0.5, step: 0.01 },
                { path: "risk.regime_filter_enabled", label: "Enable market regime filter",
                  help: "Let the bot classify the market as trending vs. choppy and use that in its decisions.",
                  kind: "toggle" },
                { path: "risk.regime_block_chop", label: "Block new trades in chop",
                  help: "Refuse new entries when the market is choppy/sideways. Chop is where trend-followers die — this saves you from a thousand small losses.",
                  kind: "toggle" },
                { path: "risk.use_kelly_sizing", label: "Kelly criterion sizing",
                  help: "Size positions by your recent win rate. Bigger trades when you've been winning, smaller when losing. Off = always use the max.",
                  kind: "toggle" },
            ],
        },
        {
            id: "exits",
            title: "Exits & Profit-Taking",
            description: "When to lock in winners and cut losers.",
            fields: [
                { path: "risk.take_profit_pct", label: "Take-profit target",
                  help: "Close the whole position when it's up by this much from entry.",
                  kind: "pct", min: 0.005, max: 0.2, step: 0.005 },
                { path: "risk.trailing_stop_pct", label: "Trailing stop distance",
                  help: "After a position goes positive, if it falls back from its peak by this much, exit. Tighter = lock in faster; looser = give winners room to run.",
                  kind: "pct", min: 0.005, max: 0.1, step: 0.005 },
                { path: "risk.tp_ladder_pct", label: "Take-profit ladder trigger",
                  help: "When a winner reaches this percent up, sell a slice and let the rest run. This is how you keep some chips off the table while still riding parabolic moves.",
                  kind: "pct", min: 0.005, max: 0.2, step: 0.005 },
                { path: "risk.tp_ladder_fraction", label: "Take-profit ladder size",
                  help: "What fraction of the position to sell when the ladder triggers. 0.5 = sell half, keep half running. 1.0 = sell everything (no runner). 0 = disable laddering.",
                  kind: "pct", min: 0, max: 1, step: 0.05 },
            ],
        },
        {
            id: "weights",
            title: "Signal Weights",
            description: "How much each input matters in the bot's verdict. Each group must sum to exactly 1.0.",
            fields: [
                { path: "weights.timeframes", label: "Timeframe weights",
                  help: "How much each candlestick interval influences the decision. Default leans on longer timeframes (15m, 1h) to filter noise. The auto-balance button rescales to sum exactly 1.0.",
                  kind: "weights", keys: ["5s", "1m", "15m", "1h"] },
                { path: "weights.signals", label: "Signal type weights",
                  help: "How much each signal family matters. TA = chart patterns (RSI/MACD/etc.), Microstructure = order book and flow, Onchain = whale movements and DEX activity, Whale-follow = mirror specific wallets.",
                  kind: "weights", keys: ["ta", "microstructure", "onchain", "whale_follow"] },
            ],
        },
        {
            id: "watchlist",
            title: "Watchlist",
            description: "Which pairs the bot watches and trades.",
            fields: [
                { path: "watchlist", label: "Trading pairs",
                  help: "Quote currency stays the same across all entries (USDC). Add or remove the base tokens you want the bot to consider. Restart required.",
                  kind: "watchlist", restartRequired: true },
            ],
        },
        {
            id: "whales",
            title: "Whale Tracker",
            description: "Mirror trades from a curated list of profitable Solana wallets via Helius.",
            fields: [
                { path: "whales.enabled", label: "Enable whale tracking",
                  help: "When on, the bot fetches recent swaps for each watched wallet every cycle and feeds them into the composite via the whale_follow signal weight.",
                  kind: "toggle" },
                { path: "whales.wallets", label: "Watched wallets",
                  help: "Solana addresses to mirror. The label is just a memo for you. Pick wallets with documented multi-month track records on Birdeye or Dune — anonymous Twitter alpha is noise.",
                  kind: "wallets" },
                { path: "whales.lookback_minutes", label: "Lookback window",
                  help: "How far back to consider whale activity. Older swaps still factor in but with exponential decay (see half-life below).",
                  kind: "slider", min: 5, max: 240, step: 5, unit: "min" },
                { path: "whales.decay_half_life_minutes", label: "Recency decay half-life",
                  help: "Time after which a swap's weight is halved. Shorter = react only to very recent activity; longer = persistent memory of recent positioning.",
                  kind: "slider", min: 1, max: 60, step: 1, unit: "min" },
                { path: "whales.per_wallet_swap_limit", label: "Swaps per wallet to fetch",
                  help: "Number of recent swaps pulled per wallet per cycle. Higher = more historical context but more Helius bandwidth.",
                  kind: "stepper", min: 5, max: 100, step: 5 },
            ],
        },
        {
            id: "wallet",
            title: "Wallet",
            description: "Starting capital and gas reserve.",
            fields: [
                { path: "app.starting_capital_usd", label: "Starting USDC capital",
                  help: "How much USDC the bot starts with. Only used on a fresh run — resumed sessions keep their actual balance.",
                  kind: "money", min: 5, max: 10000, step: 1, restartRequired: true },
                { path: "app.starting_sol_balance", label: "Starting SOL for gas",
                  help: "How much SOL is reserved for transaction fees. ~0.05 SOL is enough for ~10,000 swaps at zero priority fee.",
                  kind: "number", min: 0.001, max: 1.0, step: 0.001, unit: "SOL", restartRequired: true },
                { path: "app.priority_fee_microlamports", label: "Priority fee tip",
                  help: "Extra fee paid to Solana validators to prioritize your trades during congestion. 0 = free but may not land; 100k+ = costs pennies but reliably lands.",
                  kind: "stepper", min: 0, max: 5000000, step: 10000, unit: "µ-lamports" },
            ],
        },
        {
            id: "system",
            title: "System",
            description: "Plumbing. Don't touch unless you know what you're doing.",
            advanced: true,
            fields: [
                { path: "app.rpc_url", label: "Solana RPC URL",
                  help: "Endpoint the bot queries for chain state.",
                  kind: "text", advanced: true },
                { path: "app.jupiter_base_url", label: "Jupiter API base URL",
                  help: "Aggregator endpoint for quotes and swaps.",
                  kind: "text", advanced: true },
                { path: "app.jupiter_rate_limit_rps", label: "Jupiter rate budget",
                  help: "Sustained requests per second to Jupiter. The free lite-api ceiling is ~1.0.",
                  kind: "number", min: 0.1, max: 10, step: 0.1, unit: "rps", advanced: true },
                { path: "app.jupiter_rate_limit_burst", label: "Jupiter burst capacity",
                  help: "Token bucket size for short bursts above the sustained rate.",
                  kind: "stepper", min: 1, max: 50, step: 1, advanced: true },
                { path: "app.jupiter_max_429_retries", label: "Jupiter 429 retries",
                  help: "How many times to retry when Jupiter rate-limits us before giving up.",
                  kind: "stepper", min: 0, max: 10, step: 1, advanced: true },
                { path: "app.birdeye_api_key_env", label: "Birdeye API key",
                  help: "Free key from birdeye.so (Sign in → Developer → API Keys). When set, the bot uses one batched Birdeye call per cycle for live mark prices instead of N Jupiter probes — frees the Jupiter rate budget so you can watch many more tokens. Paste the literal key OR an env var name (uppercase).",
                  kind: "text", advanced: true, restartRequired: true },
                { path: "app.birdeye_base_url", label: "Birdeye base URL",
                  help: "Public Birdeye API endpoint. Don't change unless you have a paid plan with a different host.",
                  kind: "text", advanced: true, restartRequired: true },
                { path: "app.dashboard_host", label: "Dashboard host",
                  help: "Bind address for the dashboard HTTP server.",
                  kind: "text", advanced: true, restartRequired: true },
                { path: "app.dashboard_port", label: "Dashboard port",
                  help: "Port for the dashboard HTTP server.",
                  kind: "stepper", min: 1024, max: 65535, step: 1, advanced: true, restartRequired: true },
                { path: "app.confirmation_timeout_s", label: "Tx confirmation timeout",
                  help: "Real mode only: how long to wait for a Solana transaction to confirm before timing out.",
                  kind: "number", min: 5, max: 120, step: 1, unit: "s", advanced: true },
                { path: "app.log_level", label: "Log level",
                  help: "How verbose the structured log output is.",
                  kind: "select", options: ["DEBUG", "INFO", "WARNING", "ERROR"], advanced: true },
            ],
        },
    ];

    // ---- state ----
    let cfgServer = null;   // last-known server config (clean baseline)
    let cfgEdited = null;   // local mutable copy with user edits

    // ---- path helpers ----
    function getPath(obj, path) {
        return path.split(".").reduce((o, k) => (o == null ? undefined : o[k]), obj);
    }
    function setPath(obj, path, value) {
        const keys = path.split(".");
        const last = keys.pop();
        let cur = obj;
        for (const k of keys) {
            if (cur[k] == null || typeof cur[k] !== "object") cur[k] = {};
            cur = cur[k];
        }
        cur[last] = value;
    }
    function deepEqual(a, b) { return JSON.stringify(a) === JSON.stringify(b); }
    function deepClone(o) { return JSON.parse(JSON.stringify(o)); }

    // ---- status pill ----
    function setSettingsStatus(state, text) {
        const pill = document.getElementById("settings-status-pill");
        const txt = document.getElementById("settings-status-text");
        if (pill) pill.dataset.state = state;
        if (txt) txt.textContent = text;
    }

    // ---- dirty / validation ----
    function fieldIsDirty(path) {
        if (!cfgServer || !cfgEdited) return false;
        return !deepEqual(getPath(cfgServer, path), getPath(cfgEdited, path));
    }
    function anyDirty() {
        return !deepEqual(cfgServer, cfgEdited);
    }
    function validate() {
        const errors = [];
        if (!cfgEdited) return errors;
        // weight groups must sum to 1
        for (const sec of SETTINGS_SCHEMA) {
            for (const f of sec.fields) {
                if (f.kind === "weights") {
                    const obj = getPath(cfgEdited, f.path) || {};
                    const sum = (f.keys || []).reduce((acc, k) => acc + (Number(obj[k]) || 0), 0);
                    if (Math.abs(sum - 1.0) > 1e-4) {
                        errors.push(`${f.label}: weights sum to ${sum.toFixed(4)}, must equal 1.0`);
                    }
                }
            }
        }
        // size_min <= size_max
        const smin = Number(getPath(cfgEdited, "risk.per_trade_size_min"));
        const smax = Number(getPath(cfgEdited, "risk.per_trade_size_max"));
        if (smin > smax) errors.push("Min position size must be <= max position size");
        // watchlist non-empty
        const entries = getPath(cfgEdited, "watchlist.entries");
        if (Array.isArray(entries) && entries.length === 0) {
            errors.push("Watchlist needs at least one trading pair");
        }
        return errors;
    }

    function refreshDirtyState() {
        // mark each setting row
        document.querySelectorAll("#settings-sections-container .setting").forEach((row) => {
            const path = row.dataset.path;
            if (path) row.dataset.dirty = fieldIsDirty(path) ? "true" : "false";
        });
        const errs = validate();
        const valEl = document.getElementById("settings-validation");
        if (valEl) {
            if (errs.length === 0) {
                valEl.hidden = true;
                valEl.innerHTML = "";
            } else {
                valEl.hidden = false;
                valEl.innerHTML = `<strong>Fix before saving:</strong><ul>${errs.map((e) => `<li>${e}</li>`).join("")}</ul>`;
            }
        }
        const dirty = anyDirty();
        const saveBtn = document.getElementById("settings-save");
        const discardBtn = document.getElementById("settings-discard");
        if (saveBtn) saveBtn.disabled = !dirty || errs.length > 0;
        if (discardBtn) discardBtn.disabled = !dirty;
        if (dirty) setSettingsStatus("dirty", "unsaved changes");
        else setSettingsStatus("sync", "in sync");
    }

    // ---- field renderers ----
    function escAttr(s) { return String(s).replace(/"/g, "&quot;"); }
    function formatPct(v) { return (Number(v) * 100).toFixed(2) + "%"; }

    function renderSlider(field, value) {
        const v = Number(value);
        const valueText = field.kind === "pct" ? formatPct(v) : (v.toFixed(field.step < 1 ? 2 : 0));
        const unit = field.unit ? `<span class="value-unit">${field.unit}</span>` : "";
        return `<div class="setting-slider-wrap">
            <input type="range" class="setting-slider" data-input="slider"
                   min="${field.min}" max="${field.max}" step="${field.step}" value="${v}" />
            <span class="setting-value-badge" data-display="value">${valueText}${unit}</span>
        </div>
        <div class="setting-meta">
            <span>${field.kind === "pct" ? formatPct(field.min) : field.min}</span>
            <span>${field.kind === "pct" ? formatPct(field.max) : field.max}${field.unit ? " " + field.unit : ""}</span>
        </div>`;
    }
    function renderPct(field, value) { return renderSlider(field, value); }

    function renderStepper(field, value) {
        const unit = field.unit ? `<span class="setting-input-suffix">${field.unit}</span>` : "";
        return `<div class="setting-input-wrap">
            <button type="button" class="setting-stepper-btn" data-step="-1">−</button>
            <input type="number" class="setting-number-input" data-input="number"
                   min="${field.min}" max="${field.max}" step="${field.step}" value="${value}" />
            <button type="button" class="setting-stepper-btn" data-step="1">+</button>
            ${unit}
        </div>`;
    }

    function renderNumber(field, value) {
        const unit = field.unit ? `<span class="setting-input-suffix">${field.unit}</span>` : "";
        return `<div class="setting-input-wrap">
            <input type="number" class="setting-number-input" data-input="number"
                   min="${field.min}" max="${field.max}" step="${field.step}" value="${value}" />
            ${unit}
        </div>`;
    }

    function renderMoney(field, value) {
        return `<div class="setting-input-wrap">
            <span class="setting-input-suffix">$</span>
            <input type="number" class="setting-number-input" data-input="number"
                   min="${field.min}" max="${field.max}" step="${field.step}" value="${value}" />
        </div>`;
    }

    function renderToggle(_field, value) {
        const on = !!value;
        return `<div class="setting-toggle-wrap">
            <span class="setting-toggle-state" data-display="value" data-on="${on}">${on ? "on" : "off"}</span>
            <button type="button" class="setting-toggle-switch" data-input="toggle" data-on="${on}" aria-pressed="${on}"></button>
        </div>`;
    }

    function renderText(_field, value) {
        return `<input type="text" class="setting-text-input" data-input="text" value="${escAttr(value || "")}" />`;
    }

    function renderSelect(field, value) {
        const buttons = field.options.map((opt) => {
            const active = String(value) === String(opt) ? "true" : "false";
            return `<button type="button" data-option="${escAttr(opt)}" data-active="${active}">${opt}</button>`;
        }).join("");
        return `<div class="setting-segmented" data-input="select">${buttons}</div>`;
    }

    function renderWeights(field, value) {
        const obj = value || {};
        const sum = field.keys.reduce((s, k) => s + (Number(obj[k]) || 0), 0);
        const ok = Math.abs(sum - 1.0) < 1e-4;
        const rows = field.keys.map((k) => {
            const v = Number(obj[k]) || 0;
            return `<div class="weights-row" data-weight-key="${k}">
                <span class="weights-key">${k}</span>
                <input type="range" class="setting-slider" data-input="weight"
                       min="0" max="1" step="0.01" value="${v}" />
                <span class="weights-value" data-display="value">${(v * 100).toFixed(0)}%</span>
            </div>`;
        }).join("");
        return `<div class="weights-group" data-input="weights" data-keys="${field.keys.join(",")}">
            ${rows}
            <div class="weights-summary">
                <span class="weights-total" data-display="total" data-ok="${ok}">total: ${(sum * 100).toFixed(0)}%</span>
                <button type="button" class="weights-balance-btn" data-action="balance">↻ balance to 100%</button>
            </div>
        </div>`;
    }

    function renderWallets(_field, value) {
        const wallets = Array.isArray(value) ? value : [];
        const rows = wallets.map((w, i) => `
            <div class="watchlist-pair" data-wallet-index="${i}" style="grid-template-columns: minmax(0, 2fr) minmax(0, 1fr) 32px;">
                <input type="text" data-key="address" value="${escAttr(w.address || "")}" placeholder="Solana wallet address (base58)" />
                <input type="text" data-key="label" value="${escAttr(w.label || "")}" placeholder="memo / label" />
                <button type="button" class="watchlist-pair-remove" data-action="remove" title="Remove">×</button>
            </div>`).join("");
        return `<div class="watchlist-editor" data-input="wallets">
            <div class="watchlist-pairs">${rows}</div>
            <button type="button" class="watchlist-add" data-action="add">+ Add wallet</button>
        </div>`;
    }

    function renderWatchlist(_field, value) {
        const wl = value || { quote_symbol: "USDC", quote_mint: "", entries: [] };
        const pairs = (wl.entries || []).map((e, i) => `
            <div class="watchlist-pair" data-pair-index="${i}">
                <input type="text" data-key="symbol" value="${escAttr(e.symbol)}" placeholder="SOL" />
                <input type="text" data-key="mint" value="${escAttr(e.mint)}" placeholder="mint address (base58)" />
                <input type="number" data-key="decimals" value="${e.decimals}" min="0" max="18" />
                <button type="button" class="watchlist-pair-remove" data-action="remove" title="Remove">×</button>
            </div>`).join("");
        return `<div class="watchlist-editor" data-input="watchlist">
            <div class="watchlist-quote">
                <span class="watchlist-quote-label">Quote</span>
                <input type="text" data-key="quote_mint" value="${escAttr(wl.quote_mint)}" placeholder="USDC mint address" />
            </div>
            <div class="watchlist-pairs">${pairs}</div>
            <button type="button" class="watchlist-add" data-action="add">+ Add trading pair</button>
        </div>`;
    }

    function renderField(field) {
        const value = getPath(cfgEdited, field.path);
        let control = "";
        switch (field.kind) {
            case "slider":
            case "pct":      control = renderPct(field, value); break;
            case "stepper":  control = renderStepper(field, value); break;
            case "number":   control = renderNumber(field, value); break;
            case "money":    control = renderMoney(field, value); break;
            case "toggle":   control = renderToggle(field, value); break;
            case "text":     control = renderText(field, value); break;
            case "select":   control = renderSelect(field, value); break;
            case "weights":  control = renderWeights(field, value); break;
            case "watchlist":control = renderWatchlist(field, value); break;
            case "wallets":  control = renderWallets(field, value); break;
        }
        const tags = [];
        if (field.restartRequired) tags.push(`<span class="setting-tag tag-restart">restart</span>`);
        if (field.advanced) tags.push(`<span class="setting-tag tag-advanced">advanced</span>`);
        const tagHtml = tags.join("");
        const dirty = fieldIsDirty(field.path) ? "true" : "false";
        const advAttr = field.advanced ? ' data-advanced="true"' : '';
        return `<div class="setting" data-path="${field.path}" data-kind="${field.kind}" data-dirty="${dirty}"${advAttr}>
            <div class="setting-info">
                <div class="setting-label">${field.label}${tagHtml}</div>
                <div class="setting-help">${field.help}</div>
            </div>
            <div class="setting-control">${control}</div>
        </div>`;
    }

    function renderSection(section, idx) {
        const num = String(idx + 1).padStart(2, "0");
        const advAttr = section.advanced ? ' data-advanced="true"' : '';
        return `<section id="sec-${section.id}" class="settings-section"${advAttr}>
            <header class="settings-section-head">
                <span class="settings-section-num">${num}</span>
                <h3 class="settings-section-title">${section.title}</h3>
                <p class="settings-section-desc">${section.description}</p>
            </header>
            <div class="settings-section-body">
                ${section.fields.map(renderField).join("")}
            </div>
        </section>`;
    }

    function renderNav() {
        const nav = document.getElementById("settings-nav");
        if (!nav) return;
        nav.innerHTML = SETTINGS_SCHEMA.map((s, i) => {
            const advAttr = s.advanced ? ' data-advanced="true"' : '';
            return `<a href="#sec-${s.id}" class="settings-navlink" data-target="sec-${s.id}"${advAttr}>
                <span class="navlink-num">${String(i + 1).padStart(2, "0")}</span>
                <span class="navlink-label">${s.title}</span>
            </a>`;
        }).join("");
        nav.querySelectorAll(".settings-navlink").forEach((a, i) => {
            if (i === 0) a.classList.add("active");
            a.addEventListener("click", (e) => {
                e.preventDefault();
                const target = document.getElementById(a.dataset.target);
                if (target) {
                    target.scrollIntoView({ behavior: "smooth", block: "start" });
                    setActiveNav(a.dataset.target);
                }
            });
        });
    }
    function setActiveNav(id) {
        document.querySelectorAll(".settings-navlink").forEach((a) => {
            a.classList.toggle("active", a.dataset.target === id);
        });
    }

    function renderForm() {
        const container = document.getElementById("settings-sections-container");
        if (!container) return;
        container.innerHTML = SETTINGS_SCHEMA.map(renderSection).join("");
        attachFieldHandlers();
        refreshDirtyState();
    }

    // ---- input handlers ----
    function refreshControlVisuals(row, field, value) {
        // Update value badge / slider position / segmented active button without re-rendering whole row
        const badge = row.querySelector('[data-display="value"]');
        if (badge) {
            if (field.kind === "pct") badge.firstChild ? badge.innerHTML = formatPct(value) + (field.unit ? `<span class="value-unit">${field.unit}</span>` : "") : null;
            else if (field.kind === "slider") badge.innerHTML = (Number(value).toFixed(field.step < 1 ? 2 : 0)) + (field.unit ? `<span class="value-unit">${field.unit}</span>` : "");
            else if (field.kind === "toggle") {
                badge.dataset.on = String(!!value);
                badge.textContent = value ? "on" : "off";
            }
        }
        if (field.kind === "toggle") {
            const sw = row.querySelector(".setting-toggle-switch");
            if (sw) {
                sw.dataset.on = String(!!value);
                sw.setAttribute("aria-pressed", String(!!value));
            }
        }
        if (field.kind === "select") {
            row.querySelectorAll(".setting-segmented button").forEach((b) => {
                b.dataset.active = (b.dataset.option === String(value)) ? "true" : "false";
            });
        }
    }

    function findField(path) {
        for (const sec of SETTINGS_SCHEMA) {
            for (const f of sec.fields) if (f.path === path) return f;
        }
        return null;
    }

    function commit(path, value) {
        setPath(cfgEdited, path, value);
        const row = document.querySelector(`.setting[data-path="${path}"]`);
        const field = findField(path);
        if (row && field) refreshControlVisuals(row, field, value);
        refreshDirtyState();
    }

    function attachFieldHandlers() {
        // sliders / numbers / steppers / toggles / selects
        document.querySelectorAll("#settings-sections-container .setting").forEach((row) => {
            const path = row.dataset.path;
            const field = findField(path);
            if (!field) return;

            // generic slider / number
            row.querySelectorAll('[data-input="slider"], [data-input="number"]').forEach((el) => {
                el.addEventListener("input", () => {
                    let v = Number(el.value);
                    if (Number.isNaN(v)) v = field.min;
                    commit(path, v);
                });
            });
            // stepper buttons
            row.querySelectorAll('.setting-stepper-btn').forEach((btn) => {
                btn.addEventListener("click", () => {
                    const dir = Number(btn.dataset.step);
                    const cur = Number(getPath(cfgEdited, path)) || 0;
                    const next = Math.max(field.min, Math.min(field.max, cur + dir * field.step));
                    const input = row.querySelector('[data-input="number"]');
                    if (input) input.value = String(next);
                    commit(path, next);
                });
            });
            // toggle
            row.querySelectorAll('[data-input="toggle"]').forEach((sw) => {
                sw.addEventListener("click", () => {
                    const cur = !!getPath(cfgEdited, path);
                    commit(path, !cur);
                });
            });
            // text
            row.querySelectorAll('[data-input="text"]').forEach((el) => {
                el.addEventListener("input", () => commit(path, el.value));
            });
            // select segmented
            row.querySelectorAll('[data-input="select"] button').forEach((btn) => {
                btn.addEventListener("click", () => commit(path, btn.dataset.option));
            });
            // weights
            const wg = row.querySelector('[data-input="weights"]');
            if (wg) attachWeightsHandlers(row, field, wg);
            // watchlist
            const wl = row.querySelector('[data-input="watchlist"]');
            if (wl) attachWatchlistHandlers(row, field, wl);
            // wallets (whale-tracker)
            const ws = row.querySelector('[data-input="wallets"]');
            if (ws) attachWalletsHandlers(row, field, ws);
        });
    }

    function attachWalletsHandlers(row, field, container) {
        const path = field.path;
        const refresh = () => {
            const cur = getPath(cfgEdited, path) || [];
            const wrap = row.querySelector(".setting-control");
            if (wrap) {
                wrap.innerHTML = renderWallets(field, cur);
                attachWalletsHandlers(row, field, wrap.querySelector('[data-input="wallets"]'));
            }
            refreshDirtyState();
        };
        container.querySelectorAll(".watchlist-pair").forEach((pairEl) => {
            const idx = Number(pairEl.dataset.walletIndex);
            pairEl.querySelectorAll("input").forEach((input) => {
                input.addEventListener("input", () => {
                    const cur = [...((getPath(cfgEdited, path) || []))];
                    cur[idx] = { ...cur[idx], [input.dataset.key]: input.value };
                    setPath(cfgEdited, path, cur);
                    refreshDirtyState();
                });
            });
            const remove = pairEl.querySelector('[data-action="remove"]');
            if (remove) {
                remove.addEventListener("click", () => {
                    const cur = (getPath(cfgEdited, path) || []).filter((_, i) => i !== idx);
                    setPath(cfgEdited, path, cur);
                    refresh();
                });
            }
        });
        const addBtn = container.querySelector('[data-action="add"]');
        if (addBtn) {
            addBtn.addEventListener("click", () => {
                const cur = [...((getPath(cfgEdited, path) || [])), { address: "", label: "" }];
                setPath(cfgEdited, path, cur);
                refresh();
            });
        }
    }

    function attachWeightsHandlers(row, field, wg) {
        const path = field.path;
        const updateTotal = () => {
            const obj = getPath(cfgEdited, path) || {};
            const sum = field.keys.reduce((s, k) => s + (Number(obj[k]) || 0), 0);
            const tot = wg.querySelector('[data-display="total"]');
            if (tot) {
                tot.textContent = `total: ${(sum * 100).toFixed(0)}%`;
                tot.dataset.ok = String(Math.abs(sum - 1.0) < 1e-4);
            }
        };
        wg.querySelectorAll(".weights-row").forEach((wr) => {
            const k = wr.dataset.weightKey;
            const slider = wr.querySelector('input[type="range"]');
            const display = wr.querySelector('[data-display="value"]');
            slider.addEventListener("input", () => {
                const v = Number(slider.value);
                const obj = { ...(getPath(cfgEdited, path) || {}) };
                obj[k] = v;
                setPath(cfgEdited, path, obj);
                if (display) display.textContent = (v * 100).toFixed(0) + "%";
                updateTotal();
                refreshDirtyState();
            });
        });
        const balanceBtn = wg.querySelector('[data-action="balance"]');
        if (balanceBtn) {
            balanceBtn.addEventListener("click", () => {
                const obj = { ...(getPath(cfgEdited, path) || {}) };
                let sum = field.keys.reduce((s, k) => s + (Number(obj[k]) || 0), 0);
                if (sum === 0) {
                    // distribute evenly
                    const v = 1 / field.keys.length;
                    field.keys.forEach((k) => (obj[k] = v));
                } else {
                    const scale = 1 / sum;
                    field.keys.forEach((k) => (obj[k] = Math.round((Number(obj[k]) || 0) * scale * 100) / 100));
                    // round drift fixup on the largest
                    const drift = 1 - field.keys.reduce((s, k) => s + obj[k], 0);
                    if (Math.abs(drift) > 0) {
                        const biggest = field.keys.reduce((a, b) => (obj[a] >= obj[b] ? a : b));
                        obj[biggest] = Math.round((obj[biggest] + drift) * 1000) / 1000;
                    }
                }
                setPath(cfgEdited, path, obj);
                // re-render this row's weights group
                const newHtml = renderWeights(field, obj);
                const wrap = row.querySelector(".setting-control");
                if (wrap) {
                    wrap.innerHTML = newHtml;
                    attachWeightsHandlers(row, field, row.querySelector('[data-input="weights"]'));
                }
                refreshDirtyState();
            });
        }
    }

    function attachWatchlistHandlers(row, field, wl) {
        const path = field.path;
        const refresh = () => {
            const cur = getPath(cfgEdited, path) || { quote_symbol: "USDC", quote_mint: "", entries: [] };
            const wrap = row.querySelector(".setting-control");
            if (wrap) {
                wrap.innerHTML = renderWatchlist(field, cur);
                attachWatchlistHandlers(row, field, wrap.querySelector('[data-input="watchlist"]'));
            }
            refreshDirtyState();
        };
        const quoteInput = wl.querySelector('input[data-key="quote_mint"]');
        if (quoteInput) {
            quoteInput.addEventListener("input", () => {
                const cur = { ...(getPath(cfgEdited, path) || {}) };
                cur.quote_mint = quoteInput.value;
                setPath(cfgEdited, path, cur);
                refreshDirtyState();
            });
        }
        wl.querySelectorAll(".watchlist-pair").forEach((pairEl) => {
            const idx = Number(pairEl.dataset.pairIndex);
            pairEl.querySelectorAll("input").forEach((input) => {
                input.addEventListener("input", () => {
                    const cur = { ...(getPath(cfgEdited, path) || {}) };
                    cur.entries = [...(cur.entries || [])];
                    cur.entries[idx] = { ...cur.entries[idx] };
                    const k = input.dataset.key;
                    cur.entries[idx][k] = (k === "decimals") ? Number(input.value) : input.value;
                    setPath(cfgEdited, path, cur);
                    refreshDirtyState();
                });
            });
            const remove = pairEl.querySelector('[data-action="remove"]');
            if (remove) {
                remove.addEventListener("click", () => {
                    const cur = { ...(getPath(cfgEdited, path) || {}) };
                    cur.entries = (cur.entries || []).filter((_, i) => i !== idx);
                    setPath(cfgEdited, path, cur);
                    refresh();
                });
            }
        });
        const addBtn = wl.querySelector('[data-action="add"]');
        if (addBtn) {
            addBtn.addEventListener("click", () => {
                const cur = { ...(getPath(cfgEdited, path) || {}) };
                cur.entries = [...(cur.entries || []), { symbol: "", mint: "", decimals: 9 }];
                setPath(cfgEdited, path, cur);
                refresh();
            });
        }
    }

    // ---- scroll-spy on the nav rail ----
    function setupScrollSpy() {
        const container = document.getElementById("settings-sections-container");
        if (!container) return;
        const sections = container.querySelectorAll(".settings-section");
        container.addEventListener("scroll", () => {
            const top = container.scrollTop + 80;
            let activeId = sections[0]?.id;
            sections.forEach((s) => { if (s.offsetTop <= top) activeId = s.id; });
            if (activeId) setActiveNav(activeId);
        });
    }

    // ---- load / save / discard ----
    async function loadConfig() {
        setSettingsStatus("saving", "loading");
        try {
            const r = await fetch("/api/config");
            if (!r.ok) throw new Error(`HTTP ${r.status}`);
            const cfg = await r.json();
            cfgServer = deepClone(cfg);
            cfgEdited = deepClone(cfg);
            renderForm();
            setSettingsStatus("sync", "in sync");
            // sync raw textarea too
            const ta = document.getElementById("settings-raw-json");
            if (ta) ta.value = JSON.stringify(cfg, null, 2);
        } catch (e) {
            setSettingsStatus("error", "load failed");
            console.warn("config load failed", e);
        }
    }

    async function saveConfig() {
        const errs = validate();
        if (errs.length > 0) return;
        setSettingsStatus("saving", "saving");
        try {
            const r = await fetch("/api/config", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify(cfgEdited),
            });
            const body = await r.json().catch(() => ({}));
            if (!r.ok) {
                setSettingsStatus("error", "rejected");
                const valEl = document.getElementById("settings-validation");
                if (valEl) {
                    valEl.hidden = false;
                    valEl.innerHTML = `<strong>Server rejected the config:</strong><div class="mono" style="margin-top:6px">${body.error || `HTTP ${r.status}`}</div>`;
                }
                return;
            }
            cfgServer = deepClone(cfgEdited);
            refreshDirtyState();
            setSettingsStatus("sync", "saved · restart to apply");
            setTimeout(() => { if (!anyDirty()) setSettingsStatus("sync", "in sync"); }, 4000);
        } catch (e) {
            setSettingsStatus("error", "request failed");
            console.warn("config save failed", e);
        }
    }

    function discardChanges() {
        if (!cfgServer) return;
        cfgEdited = deepClone(cfgServer);
        renderForm();
        setSettingsStatus("sync", "in sync");
    }

    // ---- raw mode ----
    function toggleRawMode(on) {
        const sections = document.getElementById("settings-sections-container");
        const rawC = document.getElementById("settings-raw-container");
        const valEl = document.getElementById("settings-validation");
        if (on) {
            const ta = document.getElementById("settings-raw-json");
            if (ta && cfgEdited) ta.value = JSON.stringify(cfgEdited, null, 2);
            if (sections) sections.hidden = true;
            if (rawC) rawC.hidden = false;
            if (valEl) valEl.hidden = true;
        } else {
            if (sections) sections.hidden = false;
            if (rawC) rawC.hidden = true;
            refreshDirtyState();
        }
    }

    function applyRawToForm() {
        const ta = document.getElementById("settings-raw-json");
        const status = document.getElementById("settings-raw-status");
        if (!ta) return;
        try {
            const parsed = JSON.parse(ta.value);
            cfgEdited = parsed;
            if (status) { status.textContent = "applied to form"; status.className = "status mono ok"; }
            renderForm();
            // stay in raw mode but reflect dirty state
            refreshDirtyState();
        } catch (e) {
            if (status) { status.textContent = `invalid JSON: ${e.message}`; status.className = "status mono err"; }
        }
    }

    function setupSettingsHandlers() {
        const reload = document.getElementById("settings-reload");
        const save = document.getElementById("settings-save");
        const discard = document.getElementById("settings-discard");
        const advToggle = document.getElementById("settings-toggle-advanced");
        const rawToggle = document.getElementById("settings-toggle-raw");
        const pane = document.getElementById("tab-settings");
        const rawFmt = document.getElementById("settings-raw-format");
        const rawApply = document.getElementById("settings-raw-apply");

        if (reload) reload.addEventListener("click", loadConfig);
        if (save) save.addEventListener("click", saveConfig);
        if (discard) discard.addEventListener("click", discardChanges);
        if (advToggle) advToggle.addEventListener("change", () => {
            if (pane) pane.dataset.showAdvanced = advToggle.checked ? "true" : "false";
        });
        if (rawToggle) rawToggle.addEventListener("change", () => {
            toggleRawMode(rawToggle.checked);
        });
        if (rawFmt) rawFmt.addEventListener("click", () => {
            const ta = document.getElementById("settings-raw-json");
            if (!ta) return;
            try { ta.value = JSON.stringify(JSON.parse(ta.value), null, 2); } catch (_e) { /* noop */ }
        });
        if (rawApply) rawApply.addEventListener("click", applyRawToForm);

        renderNav();
        setupScrollSpy();
    }

    // ---------- backtest ----------
    let btChart = null;

    function ensureBtChart() {
        if (btChart) return btChart;
        const ctx = document.getElementById("bt-chart").getContext("2d");
        btChart = new Chart(ctx, {
            type: "line",
            data: { labels: [], datasets: [{
                data: [], borderColor: "#f5b13a",
                borderWidth: 1.6, tension: 0.2,
                pointRadius: 0, fill: false,
            }]},
            options: {
                responsive: true, maintainAspectRatio: false, animation: false,
                scales: {
                    x: { ticks: { color: "#5b6473", maxTicksLimit: 8,
                                  font: { family: "IBM Plex Mono, monospace", size: 10 } },
                         grid: { color: "rgba(255,255,255,0.03)" } },
                    y: { ticks: { color: "#5b6473",
                                  font: { family: "IBM Plex Mono, monospace", size: 10 },
                                  callback: v => "$" + Number(v).toFixed(2) },
                         grid: { color: "rgba(255,255,255,0.04)" } },
                },
                plugins: { legend: { display: false } },
            },
        });
        return btChart;
    }

    function renderBtResult(r) {
        const meta = document.getElementById("bt-result-meta");
        meta.textContent = `${r.id} · ${r.pair} · ${r.bars_processed} bars`;

        const m = document.getElementById("bt-metrics");
        const ret = r.total_return_pct;
        const dd = r.max_drawdown_pct;
        const retCls = ret > 0 ? "pos" : ret < 0 ? "neg" : "";
        m.innerHTML = `
            <div class="bt-metric"><span class="bt-metric-label">Return</span>
                <span class="bt-metric-value ${retCls}">${(ret*100).toFixed(2)}%</span></div>
            <div class="bt-metric"><span class="bt-metric-label">Final eq.</span>
                <span class="bt-metric-value">$${r.final_equity.toFixed(2)}</span></div>
            <div class="bt-metric"><span class="bt-metric-label">Sharpe</span>
                <span class="bt-metric-value">${r.sharpe.toFixed(2)}</span></div>
            <div class="bt-metric"><span class="bt-metric-label">Max DD</span>
                <span class="bt-metric-value neg">${(dd*100).toFixed(2)}%</span></div>
            <div class="bt-metric"><span class="bt-metric-label">Trades</span>
                <span class="bt-metric-value">${r.n_trades}</span></div>
            <div class="bt-metric"><span class="bt-metric-label">Win/Loss</span>
                <span class="bt-metric-value">${r.n_wins}W / ${r.n_losses}L</span></div>
        `;

        const c = ensureBtChart();
        c.data.labels = r.equity_curve.map(p => p.t.slice(11, 19));
        c.data.datasets[0].data = r.equity_curve.map(p => p.e);
        c.update("none");

        const tbody = document.querySelector("#bt-trades tbody");
        tbody.innerHTML = r.trades.slice(0, 50).map(t => {
            const cls = t.side === "buy" ? "side-buy" : "side-sell";
            return `<tr>
                <td class="col-time">${(t.timestamp || "").slice(0,19).replace("T"," ")}</td>
                <td class="${cls}">${t.side.toUpperCase()}</td>
                <td class="num">${Number(t.base_amount).toFixed(6)}</td>
                <td class="num">${Number(t.price).toFixed(4)}</td>
                <td class="num">${Number(t.quote_amount).toFixed(4)}</td>
            </tr>`;
        }).join("");
    }

    async function loadBtHistory() {
        try {
            const r = await fetch("/api/backtest/history");
            const list = await r.json();
            const tbody = document.querySelector("#bt-history tbody");
            const empty = document.getElementById("bt-history-empty");
            if (!list || list.length === 0) {
                tbody.innerHTML = "";
                empty.style.display = "";
                return;
            }
            empty.style.display = "none";
            tbody.innerHTML = list.map(h => {
                const ret = h.total_return_pct;
                const cls = ret > 0 ? "pnl-pos" : ret < 0 ? "pnl-neg" : "";
                return `<tr data-id="${h.id}" class="bt-history-row">
                    <td class="mono">${h.id}</td>
                    <td>${h.pair}</td>
                    <td class="num ${cls}">${(ret*100).toFixed(2)}%</td>
                    <td class="num">${h.sharpe.toFixed(2)}</td>
                    <td class="num">${h.n_trades}</td>
                    <td class="num pnl-neg">${(h.max_drawdown_pct*100).toFixed(2)}%</td>
                    <td class="mono">${(h.completed_at || "").slice(0,19).replace("T"," ")}</td>
                </tr>`;
            }).join("");
            // Click a row to load that result
            tbody.querySelectorAll(".bt-history-row").forEach(tr => {
                tr.addEventListener("click", async () => {
                    const r = await fetch(`/api/backtest/result/${tr.dataset.id}`);
                    if (r.ok) renderBtResult(await r.json());
                });
            });
        } catch (e) { /* noop */ }
    }

    async function runBacktest(e) {
        e.preventDefault();
        const status = document.getElementById("bt-status");
        const file = document.getElementById("bt-csv").files[0];
        if (!file) { status.textContent = "select a CSV first"; status.className = "status mono err"; return; }
        const params = {
            pair: document.getElementById("bt-pair").value,
            timeframe: document.getElementById("bt-timeframe").value,
            starting_cash: Number(document.getElementById("bt-cash").value),
            fee_bps: Number(document.getElementById("bt-fee").value),
            slippage_bps: Number(document.getElementById("bt-slip").value),
            warmup_bars: Number(document.getElementById("bt-warmup").value),
            entry_threshold: Number(document.getElementById("bt-entry").value),
            exit_flip_threshold: Number(document.getElementById("bt-exit").value),
        };
        const fd = new FormData();
        fd.append("csv_file", file);
        fd.append("params", JSON.stringify(params));
        status.textContent = "running…"; status.className = "status mono";
        try {
            const r = await fetch("/api/backtest/run", { method: "POST", body: fd });
            const body = await r.json();
            if (!r.ok) { status.textContent = `error: ${body.detail || r.status}`; status.className = "status mono err"; return; }
            status.textContent = `done · ${body.bars_processed} bars · ${body.n_trades} trades`;
            status.className = "status mono ok";
            renderBtResult(body);
            loadBtHistory();
        } catch (err) {
            status.textContent = `request failed: ${err.message}`; status.className = "status mono err";
        }
    }

    // ---------- bootstrap ----------
    document.addEventListener("DOMContentLoaded", () => {
        initChart();
        setupTabs();
        setupControls();
        setupSettingsHandlers();
        setupSellModal();

        const btForm = document.getElementById("bt-form");
        if (btForm) btForm.addEventListener("submit", runBacktest);

        // Bootstrap snapshot from REST while WS connects
        fetch("/api/state")
            .then((r) => r.json())
            .then((snap) => {
                if (snap && snap.mode) render(snap);
            })
            .catch(() => { /* noop */ });

        connect();
    });
})();
