#!/usr/bin/env python3
"""Summarize every measured cell; charts use medians and observed trial ranges."""
import csv
import json
from collections import defaultdict
from pathlib import Path
import statistics
import shutil
import sys

root = Path(__file__).resolve().parents[1]
source = Path(sys.argv[1]) if len(sys.argv) > 1 else root / 'bench-results/external'
out = root / 'docs/benchmark-results'
out.mkdir(parents=True, exist_ok=True)
for path in source.glob('*-metadata.json'):
    shutil.copyfile(path, out / path.name)
for name in ['Cargo.lock', 'working-tree.patch']:
    if (source / name).exists(): shutil.copyfile(source / name, out / name)
criterion = {}
for path in (root / 'target/criterion').glob('**/new/benchmark.json'):
    benchmark = json.loads(path.read_text())
    key = benchmark['full_id']
    if key.startswith(('external/', 'tax/')):
        criterion[key] = {name: json.loads((path.parent / (name + '.json')).read_text()) for name in ['benchmark', 'estimates', 'sample']}
if criterion:
    if len(criterion) != 236: raise RuntimeError('Run both complete Criterion suites before publishing')
    (out / 'criterion.json').write_text(json.dumps(criterion, indent=2))
    with (out / 'criterion-summary.csv').open('w') as f:
        w = csv.writer(f, lineterminator='\n')
        w.writerow(['benchmark', 'median_ns', 'mean_ns', 'slope_ns'])
        for key, result in sorted(criterion.items()):
            w.writerow([key, *[result['estimates'][name]['point_estimate'] if result['estimates'][name] else None for name in ['median', 'mean', 'slope']]])
groups = defaultdict(list)
for suite in ['seq', 'concurrent']:
    path = source / f'{suite}.csv'
    if not path.exists(): raise RuntimeError(f'Missing required suite: {path}')
    rows = list(csv.DictReader(path.open()))
    expected = 1600 if suite == 'seq' else 7840
    if len(rows) != expected:
        raise RuntimeError(f'{suite}: expected {expected} rows for the full five-trial matrix, got {len(rows)}')
    (out / path.name).write_text(path.read_text())
    for r in rows:
        key = (suite, r['candidate'], r['dataset'], r['operation'], r['optimized'], int(r['threads']))
        groups[key].append(int(r['ops']) / float(r['seconds']) / 1e6)
with (out / 'summary.csv').open('w') as f:
    w = csv.writer(f, lineterminator='\n')
    w.writerow(['suite', 'candidate', 'dataset', 'operation', 'optimized', 'threads', 'samples', 'median_mops', 'min_mops', 'max_mops'])
    for key, values in sorted(groups.items()):
        if len(values) != 5: raise RuntimeError(f'Incomplete trial group: {key}')
        w.writerow([*key, len(values), statistics.median(values), min(values), max(values)])

def value(suite, candidate, dataset, op, threads=1):
    v = groups.get((suite, candidate, dataset, op, 'true', threads))
    return statistics.median(v) if v else None

lines = ['# External comparison results', '', 'Generated from individual trials by `scripts/report_comparison.py`. Throughput is millions of operations per second; ranges are the observed minimum and maximum across trials.', '', '## Baseline B: sequential lookup, 50% hits', '', '| Dataset | Ours Mops/s | roaring Mops/s | croaring Mops/s | vs roaring | vs croaring |', '|---|---:|---:|---:|---:|---:|']
for d in ['dense', 'sparse', 'clustered', 'balanced']:
    values = [value('seq', c, d, 'contains_50') for c in ['ours', 'roaring', 'croaring']]
    if all(values):
        a,b,c = values
        lines.append(f'| {d} | {a:.2f} | {b:.2f} | {c:.2f} | {a/b:.2f}× | {a/c:.2f}× |')
lines += ['', '![Sequential lookup throughput with observed trial ranges](benchmark-results/reads.png)', '', '## Baseline C: concurrent pure reads, 8 threads', '', '| Dataset | sharded | snapshot | epoch | sharded roaring | sharded croaring | DashSet |', '|---|---:|---:|---:|---:|---:|---:|']
for d in ['dense', 'sparse', 'clustered', 'balanced']:
    values = [value('concurrent', c, d, 'read200_uniform_change', 8) for c in ['sharded', 'snapshot', 'epoch', 'sharded_roaring', 'sharded_croaring', 'dashset']]
    if all(values): lines.append('| ' + d + ' | ' + ' | '.join(f'{x:.2f}' for x in values) + ' |')
lines += ['', '![Concurrent pure-read scaling](benchmark-results/concurrent-reads.png)', '', 'The full [summary CSV](benchmark-results/summary.csv) includes all wins and losses, unoptimized reads, all hit ratios, build, optimize, removal, set overlap/cardinality ratios, and concurrent mixes. Individual trials are archived alongside it. See [methodology and interpretation](external-benchmarks.md) before using a number in a resume.', '']
memory = source / 'memory.csv'
if memory.exists():
    (out / memory.name).write_text(memory.read_text())
    pairs = defaultdict(dict)
    for r in csv.DictReader(memory.open()):
        pairs[(r['candidate'], r['dataset'], r['sample'])][int(r['copies'])] = r
    mem = defaultdict(list)
    for (candidate, dataset, _), copies in pairs.items():
        for n, r in copies.items():
            if n and 0 in copies:
                base = copies[0]
                mem[(candidate, dataset, n)].append((int(r['rss_kib']), int(r['rss_kib'])-int(base['rss_kib']), int(r['peak_rss_bytes'])))
    with (out / 'memory-summary.csv').open('w') as f:
        w = csv.writer(f, lineterminator='\n')
        w.writerow(['candidate','dataset','copies','samples','median_rss_kib','median_delta_kib','min_delta_kib','max_delta_kib','median_peak_rss_bytes'])
        for key, rows in sorted(mem.items()):
            if len(rows) != 5: raise RuntimeError(f'Incomplete memory trial group: {key}')
            rss, delta, peak = zip(*rows)
            w.writerow([*key,len(rows),statistics.median(rss),statistics.median(delta),min(delta),max(delta),statistics.median(peak)])
    lines += ['', '## Process memory', '', 'The [memory summary](benchmark-results/memory-summary.csv) reports RSS, paired empty-process RSS differences, trial ranges, and peak RSS. These include allocator retention and construction effects; they are not exact heap sizes. See the raw memory CSV for cardinalities and copy counts.', '']
    lines += ['| Dataset | Ours RSS MiB | roaring RSS MiB | croaring RSS MiB |', '|---|---:|---:|---:|']
    for d in ['dense','sparse','clustered','balanced']:
        vals=[]
        for c in ['ours','roaring','croaring']:
            v=mem.get((c,d,1))
            vals.append(statistics.median(r[0] for r in v)/1024 if v else None)
        if all(v is not None for v in vals):
            lines.append('| '+d+' | '+' | '.join(f'{v:.2f}' for v in vals)+' |')
    lines += ['', 'RSS table: one live optimized map per process, before subtracting the empty-process control. The linked CSV includes paired differences and all concurrent wrappers.', '']
(root / 'docs/external-results.md').write_text('\n'.join(lines))

import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
colors = ['#2563eb', '#e87924', '#6b7280']
fig, ax = plt.subplots(figsize=(9, 4.5))
datasets = ['dense', 'sparse', 'clustered', 'balanced']
for j,c in enumerate(['ours','roaring','croaring']):
    pos=[]; med=[]; lo=[]; hi=[]
    for i,d in enumerate(datasets):
        v=groups.get(('seq',c,d,'contains_50','true',1))
        if v:
            m=statistics.median(v);pos.append(i+(j-1)*.25);med.append(m);lo.append(m-min(v));hi.append(max(v)-m)
    ax.bar(pos,med,width=.23,label=c,color=colors[j],yerr=[lo,hi],capsize=3)
ax.set_xticks(range(4),datasets);ax.set_ylabel('Million lookups / second');ax.set_title('Sequential lookup · 50% hits · optimized containers');ax.legend();ax.grid(axis='y',alpha=.2);ax.set_axisbelow(True)
fig.tight_layout();fig.savefig(out/'reads.png',dpi=180);plt.close(fig)
if any(k[0]=='concurrent' for k in groups):
    fig, axes=plt.subplots(1,2,figsize=(11,4.5))
    for ax,d in zip(axes,['clustered','balanced']):
        for c in ['sharded','snapshot','epoch','sharded_roaring','sharded_croaring','dashset']:
            ts=[t for t in [1,2,4,8] if value('concurrent',c,d,'read200_uniform_change',t)]
            line, = ax.plot(ts,[value('concurrent',c,d,'read200_uniform_change',t) for t in ts],marker='o',label=c)
            trials=[groups[('concurrent',c,d,'read200_uniform_change','true',t)] for t in ts]
            ax.fill_between(ts,[min(v) for v in trials],[max(v) for v in trials],color=line.get_color(),alpha=.10)
        ax.set_title(d);ax.set_xlabel('Threads');ax.set_xticks([1,2,4,8]);ax.grid(alpha=.2)
    axes[0].set_ylabel('Million lookups / second');axes[1].legend(fontsize=8)
    fig.suptitle('Concurrent lookup · optimized preload · 50% stable hits');fig.tight_layout();fig.savefig(out/'concurrent-reads.png',dpi=180);plt.close(fig)
print('\n'.join(lines))
