#!/usr/bin/env python3
"""P9 graphs: read bench-results/scaling.csv, emit docs/graphs/{read_scaling,write_impact}.png."""

import csv
import math
import os

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CSV_PATH = os.path.join(ROOT, "bench-results", "scaling.csv")
OUT_DIR = os.path.join(ROOT, "docs", "graphs")

# Fixed entity->color assignment (identical in every chart; validated categorical slots).
SERIES = [
    ("sequential", "#2a78d6"),
    ("sharded", "#1baf7a"),
    ("snapshot", "#eda100"),
    ("epoch", "#008300"),
]
SURFACE = "#fcfcfb"
INK_PRIMARY = "#0b0b0b"
INK_SECONDARY = "#52514e"
INK_MUTED = "#898781"
GRIDLINE = "#e1e0d9"
BASELINE = "#c3c2b7"


def load(path):
    """-> {(structure, workload): [(threads, mops), ...] sorted by threads}"""
    data = {}
    with open(path, newline="") as f:
        for row in csv.DictReader(f):
            key = (row["structure"], row["workload"])
            data.setdefault(key, []).append((int(row["threads"]), float(row["mops"])))
    for pts in data.values():
        pts.sort()
    return data


def plot_workload(data, workload, title, out_path):
    fig, ax = plt.subplots(figsize=(8, 5), dpi=200)
    fig.set_facecolor(SURFACE)
    ax.set_facecolor(SURFACE)

    ends = []  # (final_mops, final_threads, color) for endpoint value labels
    for name, color in SERIES:
        pts = data.get((name, workload))
        if not pts:
            continue
        xs = [t for t, _ in pts]
        ys = [m for _, m in pts]
        # 2px line, >=8px marker with a 2px surface ring so markers stay legible where lines cross
        ax.plot(
            xs,
            ys,
            label=name,
            color=color,
            linewidth=2,
            solid_capstyle="round",
            solid_joinstyle="round",
            marker="o",
            markersize=8,
            markeredgecolor=SURFACE,
            markeredgewidth=2,
            zorder=3,
        )
        ends.append((ys[-1], xs[-1], color))

    # Selective direct labels: the endpoint value of each series, in text ink (never the
    # series color); nudge apart in log space where two endpoints would collide.
    ymin = min(v for v, _, _ in ends)
    ymax = max(v for v, _, _ in ends)
    span = math.log10(ymax) - math.log10(ymin) or 1.0
    ends.sort()
    aligns = ["center"] * len(ends)
    for i in range(1, len(ends)):
        gap = (math.log10(ends[i][0]) - math.log10(ends[i - 1][0])) / span
        if gap < 0.055:
            aligns[i - 1] = "top"
            aligns[i] = "bottom"
    for (val, x, _), va in zip(ends, aligns):
        label = f"{val:.2f}" if val < 1 else f"{val:.1f}"
        ax.annotate(
            label,
            (x, val),
            xytext=(10, 0),
            textcoords="offset points",
            va=va,
            ha="left",
            fontsize=9,
            color=INK_SECONDARY,
        )

    # Throughput spans ~3 orders of magnitude between sharded and the RCU types; a linear
    # axis would flatten the lock-free lines onto the x-axis.
    ax.set_yscale("log")
    ax.set_xscale("log", base=2)
    ax.set_xticks([1, 2, 4, 8])
    ax.set_xticklabels(["1", "2", "4", "8"])
    ax.set_xlim(0.85, 11.5)
    ax.minorticks_off()

    ax.set_title(title, color=INK_PRIMARY, fontsize=13, pad=14, loc="left")
    ax.set_xlabel("threads", color=INK_SECONDARY, fontsize=10)
    ax.set_ylabel("throughput, Mops/s (log scale)", color=INK_SECONDARY, fontsize=10)
    ax.tick_params(colors=INK_MUTED, labelsize=9, length=0)
    ax.grid(axis="y", color=GRIDLINE, linewidth=1, linestyle="-", zorder=0)
    for side in ("top", "right"):
        ax.spines[side].set_visible(False)
    for side in ("left", "bottom"):
        ax.spines[side].set_color(BASELINE)
        ax.spines[side].set_linewidth(1)

    legend = ax.legend(
        loc="center left",
        bbox_to_anchor=(1.06, 0.5),
        frameon=False,
        fontsize=9,
        labelcolor=INK_SECONDARY,
    )
    for line in legend.get_lines():
        line.set_linewidth(2)

    fig.tight_layout()
    fig.savefig(out_path, facecolor=SURFACE, bbox_inches="tight")
    plt.close(fig)
    print(f"wrote {os.path.relpath(out_path, ROOT)}")


def main():
    os.makedirs(OUT_DIR, exist_ok=True)
    data = load(CSV_PATH)
    plot_workload(
        data,
        "read95",
        "Read-heavy scaling — read95 (95% contains / 5% insert)",
        os.path.join(OUT_DIR, "read_scaling.png"),
    )
    plot_workload(
        data,
        "write95",
        "Write-heavy impact — write95 (5% contains / 95% insert)",
        os.path.join(OUT_DIR, "write_impact.png"),
    )


if __name__ == "__main__":
    main()
