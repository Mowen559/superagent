"""创建等哈希的评测源码副本；不复制用户数据或模型凭据。"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
REPO = ROOT.parent


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    output = Path(args.output).resolve()
    assert not output.exists(), 'snapshot receipt already exists'
    stage = Path(tempfile.mkdtemp(prefix='hub-frozen-source-'))
    paths = set()
    groups = [(ROOT/'src',{'.rs'}), (ROOT/'skills',{'.md','.py','.js','.cjs','.json'}),
              (ROOT/'scripts',{'.py','.ps1'}), (REPO/'deepcloud-ui/native',{'.js','.cjs','.json','.wasm','.sql'}),
              (REPO/'deepcloud-ui/src/lib',{'.js','.cjs','.ts','.tsx','.json'})]
    for directory, suffixes in groups:
        paths.update(p for p in directory.rglob('*') if p.is_file() and p.suffix in suffixes
                     and not {'node_modules','__pycache__','.git'} & set(p.parts))
    paths.update(ROOT/name for name in ['Cargo.toml','Cargo.lock','target/debug/hub.exe'])
    paths.update(REPO/'deepcloud-ui'/name for name in ['package.json','package-lock.json',
        'scripts/hub-rule-runtime.cjs','scripts/hub-delivery-replay.cjs','scripts/hiker-standard-template.test.cjs'])
    files = []
    for original in sorted(paths):
        before = sha(original)
        copied = stage/original.relative_to(REPO)
        copied.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(original, copied)
        assert sha(original) == before == sha(copied), 'source changed while copying: '+str(original)
        files.append({'origin':str(original),'path':str(copied),'sha256':before})
    # Recheck the entire captured set before accepting a coherent snapshot.
    assert all(sha(f['origin']) == f['sha256'] for f in files), 'concurrent edit during snapshot; retry with a new receipt'
    dependency_source = REPO/'deepcloud-ui/node_modules'
    # NODE_PATH resolves actual installed packages without putting a huge
    # traversable dependency junction inside the frozen source tree.
    result = {'schema_version':1,'stage':str(stage),'source_head':subprocess.check_output(
        ['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
        'source_diff_sha256':hashlib.sha256(subprocess.check_output(['git','diff','--binary'],cwd=ROOT)).hexdigest(),
        'files':files,'node_modules_reference':str(dependency_source),
        'credential_policy':'not copied; settings remain a separately hashed reference',
        'production_code':'byte-identical source snapshot, no replacement runtime'}
    manifest = stage/'super-agent/frozen-source.json'
    manifest.write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf-8')
    with output.open('x',encoding='utf-8') as stream:
        json.dump({'manifest':str(manifest),'sha256':sha(manifest),'stage':str(stage),'files':len(files)},stream,ensure_ascii=False,indent=2)
    print(json.dumps({'stage':str(stage),'files':len(files),'output':str(output)},ensure_ascii=False))


if __name__ == '__main__':
    main()
