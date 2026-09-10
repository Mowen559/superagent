"""Compare Rust declarations against a Git baseline using tree-sitter.

Usage: py -3 scripts/audit_module_split.py --baseline HEAD --output report.json
Requires tree-sitter==0.25.2 and tree-sitter-rust. Does not modify implementation files.
Equality ignores comments, whitespace and visibility, but preserves literal bytes.
It is source evidence, not proof of name resolution or runtime equivalence.
"""
import argparse
import collections
import hashlib
import json
import pathlib
import subprocess
import sys
import tempfile

sys.path.insert(0, str(pathlib.Path(tempfile.gettempdir()) / (
    'superagent-audit-python312' if sys.version_info[:2] == (3, 12) else 'superagent-audit-python')))
from tree_sitter import Language, Parser
import tree_sitter_rust


PARSER = Parser(Language(tree_sitter_rust.language()))
ROOT = pathlib.Path(__file__).resolve().parents[1]
GROUPS = ['src/routes/agents', 'src/services/cli_auth',
          'src/services/brain/provider_capability_runtime']


def tokens(node, source, ignore_visibility=True):
    if node.type in ('line_comment', 'block_comment'):
        return []
    if ignore_visibility and node.type == 'visibility_modifier':
        return []
    if not node.children:
        return [(node.type, source[node.start_byte:node.end_byte].decode('utf-8'))]
    return [t for c in node.children for t in tokens(c, source, ignore_visibility)]


def declarations(source, path):
    tree = PARSER.parse(source)
    items = collections.defaultdict(list)
    errors = []
    if tree.root_node.has_error:
        errors.append({'path': path, 'kind': 'tree_has_error'})

    def walk(node, scope=''):
        if node.type == 'ERROR' or node.is_missing:
            errors.append({'path': path, 'line': node.start_point.row + 1,
                           'kind': node.type})
        if node.type in ('function_item', 'struct_item', 'enum_item', 'type_item',
                         'const_item', 'static_item', 'trait_item'):
            name = node.child_by_field_name('name')
            key = scope + source[name.start_byte:name.end_byte].decode() if name else scope + '?'
            normalized = tokens(node, source)
            exact = tokens(node, source, False)
            attributes = []
            previous = node.prev_named_sibling
            while previous and previous.type in ('attribute_item', 'line_comment', 'block_comment'):
                if previous.type == 'attribute_item':
                    attributes = tokens(previous, source) + attributes
                previous = previous.prev_named_sibling
            normalized = attributes + normalized
            exact = attributes + exact
            items[node.type + ':' + key].append({
                'path': path, 'line': node.start_point.row + 1,
                'normalized': normalized, 'exact': exact,
                'sha256': hashlib.sha256(json.dumps(normalized).encode()).hexdigest()})
            return
        if node.type == 'impl_item':
            typ = node.child_by_field_name('type')
            trait = node.child_by_field_name('trait')
            scope = ((source[trait.start_byte:trait.end_byte].decode() + ' for ') if trait else '')
            scope += source[typ.start_byte:typ.end_byte].decode() + '::'
        # Split test files sometimes add an extra tests module; ignore module
        # wrappers when matching declarations, but keep paths in the report.
        for child in node.children:
            walk(child, scope)

    walk(tree.root_node)
    return items, errors


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--baseline', default='HEAD')
    ap.add_argument('--output', required=True)
    ap.add_argument('--baseline-dir', help='Read baseline files from a detached worktree (avoids subprocess on Windows).')
    args = ap.parse_args()
    commit = args.baseline if args.baseline_dir else subprocess.check_output(
        ['git', 'rev-parse', args.baseline], cwd=ROOT, text=True,
        stdin=subprocess.DEVNULL, stderr=subprocess.PIPE).strip()
    result = {'baseline': commit, 'method': __doc__, 'groups': [], 'files': {}}
    for group in GROUPS:
        before = (pathlib.Path(args.baseline_dir) / (group + '.rs')).read_bytes() if args.baseline_dir else subprocess.check_output(
            ['git', 'show', f'{commit}:{group}.rs'], cwd=ROOT,
            stdin=subprocess.DEVNULL, stderr=subprocess.PIPE)
        old, errors = declarations(before, group + '.rs')
        new = collections.defaultdict(list)
        for file in sorted((ROOT / group).glob('*.rs')):
            data = file.read_bytes()
            path = file.relative_to(ROOT).as_posix()
            result['files'][path] = hashlib.sha256(data).hexdigest()
            found, errs = declarations(data, path)
            errors.extend(errs)
            for key, entries in found.items():
                new[key].extend(entries)
        same, changed, missing, visibility = [], [], [], []
        for key, entries in old.items():
            for entry in entries:
                matches = new.get(key, [])
                match = next((x for x in matches if x['normalized'] == entry['normalized']), None)
                if match:
                    same.append({'symbol': key, 'beforeLine': entry['line'],
                                 'after': match['path'], 'afterLine': match['line']})
                    if match['exact'] != entry['exact']:
                        visibility.append(key)
                elif matches:
                    changed.append({'symbol': key, 'before': entry, 'after': matches})
                else:
                    missing.append({'symbol': key, 'beforeLine': entry['line']})
        result['groups'].append({'module': group, 'beforeDeclarations': sum(map(len, old.values())),
                                'afterDeclarations': sum(map(len, new.values())),
                                'equal': same, 'changed': changed, 'missing': missing,
                                'addedSymbols': sorted(set(new) - set(old)),
                                'duplicateSymbols': {k: len(v) for k, v in new.items() if len(v) > 1},
                                'visibilityChanged': visibility, 'parseErrors': errors})
    output = pathlib.Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding='utf-8')
    for group in result['groups']:
        print(group['module'], {k: len(group[k]) for k in
              ['equal', 'changed', 'missing', 'addedSymbols', 'visibilityChanged', 'parseErrors']})
        for key in ['changed', 'missing', 'addedSymbols', 'parseErrors']:
            if group[key]:
                print(key, [x.get('symbol', x) if isinstance(x, dict) else x for x in group[key]])


if __name__ == '__main__':
    main()
