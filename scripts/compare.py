#!/usr/bin/env python3
"""Run reproducible external comparisons; every CSV contains individual trials."""
import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / 'target/release/examples/compare'
DATASETS = ['dense', 'sparse', 'clustered', 'balanced']
SEQ = ['ours', 'roaring', 'croaring', 'hashset', 'btreeset']
CONC = ['sharded', 'snapshot', 'epoch', 'sharded_roaring', 'sharded_croaring', 'dashset']

def command(args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()

def main():
    p = argparse.ArgumentParser()
    p.add_argument('mode', choices=['seq', 'concurrent', 'memory', 'reads'])
    p.add_argument('--output', required=True)
    p.add_argument('--samples', type=int, default=5)
    p.add_argument('--candidates', nargs='+')
    p.add_argument('--copies', type=int, default=8)
    p.add_argument('--datasets', nargs='+', default=DATASETS)
    a = p.parse_args()
    # Keep recorded source hashes tied to a binary rebuilt from that checkout.
    subprocess.run(['cargo', 'build', '--release', '--example', 'compare'], cwd=ROOT, check=True)
    out = Path(a.output); out.mkdir(parents=True, exist_ok=True)
    source_files = [ROOT / 'Cargo.toml', ROOT / 'Cargo.lock'] + [f for directory in ['src', 'benches', 'examples'] for f in (ROOT / directory).rglob('*.rs')]
    metadata = {'source_sha256': {str(f.relative_to(ROOT)): hashlib.sha256(f.read_bytes()).hexdigest() for f in source_files}, 'build_env': {k: os.environ.get(k) for k in ['RUSTFLAGS', 'CFLAGS', 'ROARING_ARCH']}, 'mode': a.mode, 'command': os.sys.argv, 'platform': platform.platform(), 'rustc': command(['rustc', '-Vv']), 'commit': command(['git', 'rev-parse', 'HEAD']), 'date': time.strftime('%Y-%m-%dT%H:%M:%S%z'), 'cpu_count': os.cpu_count(), 'binary_sha256': hashlib.sha256(BINARY.read_bytes()).hexdigest()}
    (out / f'{a.mode}-metadata.json').write_text(json.dumps(metadata, indent=2))
    (out / 'Cargo.lock').write_bytes((ROOT / 'Cargo.lock').read_bytes())
    (out / 'working-tree.patch').write_text(command(['git', 'diff']))
    candidates = a.candidates or (CONC if a.mode == 'concurrent' else SEQ)
    rng = random.Random(0xFEEDBEEF)
    with (out / f'{a.mode}.csv').open('w') as f:
        writer = None
        if a.mode == 'memory':
            writer = csv.writer(f, lineterminator='\n')
            writer.writerow(['candidate', 'dataset', 'sample', 'copies', 'cardinality', 'rss_kib', 'peak_rss_bytes'])
        for sample in range(a.samples):
            for dataset in a.datasets:
                order = candidates.copy(); rng.shuffle(order)
                for candidate in order:
                    print(f'{a.mode} sample={sample} dataset={dataset} candidate={candidate}', flush=True)
                    if a.mode == 'memory':
                        # Multiple independent maps make payload differences visible above runtime overhead.
                        for copies in [0, a.copies]:
                            proc = subprocess.Popen(['/usr/bin/time', '-l', str(BINARY), 'memory', candidate, dataset, str(copies)], cwd=ROOT, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                            ready = proc.stdout.readline().split()
                            if not ready or ready[0] != 'READY':
                                raise RuntimeError(proc.communicate())
                            pid, cardinality = ready[1:]
                            rss = int(command(['/bin/ps', '-o', 'rss=', '-p', pid]))
                            _, errors = proc.communicate('\n')
                            if proc.returncode: raise RuntimeError(errors)
                            peak = next(int(line.split()[0]) for line in errors.splitlines() if 'maximum resident set size' in line)
                            writer.writerow([candidate, dataset, sample, copies, cardinality, rss, peak]); f.flush()
                    else:
                        count = '1' if a.mode == 'concurrent' else str(sample)
                        proc = subprocess.Popen([str(BINARY), a.mode, candidate, dataset, count], cwd=ROOT, stdout=subprocess.PIPE, text=True)
                        rows = csv.DictReader(proc.stdout)
                        for row in rows:
                            row['sample'] = sample
                            if writer is None:
                                writer = csv.DictWriter(f, fieldnames=rows.fieldnames, lineterminator='\n'); writer.writeheader()
                            writer.writerow(row); f.flush()
                        if proc.wait(): raise RuntimeError(f'benchmark failed: {candidate}/{dataset}')

if __name__ == '__main__':
    main()
