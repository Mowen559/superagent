"""Create an explicit coached follow-up, preserving the original attempt."""
import argparse
import copy
import json
from pathlib import Path
import shutil
import tempfile
from hub_delivery_evals import digest, save_new

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--previous', required=True)
parser.add_argument('--requests', required=True)
parser.add_argument('--feedback', required=True)
parser.add_argument('--output', required=True)
parser.add_argument('--fresh-context', action='store_true')
parser.add_argument('--clean-copy', action='store_true')
args = parser.parse_args()
previous = json.loads(Path(args.previous).read_text(encoding='utf-8'))['attempts'][0]
definition = copy.deepcopy(json.loads(Path(args.requests).read_text(encoding='utf-8'))[0])
session = next(e['session_id'] for e in previous['events'] if e['kind'] == 'trace-snapshot')
workspace = previous.get('execution_workspace') or str(Path.home()/'.superagent/runs'/previous['task_id']/'workspace')
definition.update(id='H01-coached-repair', classification='corrected', prior_exposure=True,
    based_on_task=previous['task_id'], previous_receipt_sha256=digest(args.previous), feedback_sha256=digest(args.feedback))
project = workspace
if args.clean_copy:
    project = str(Path(tempfile.mkdtemp(prefix='hub-coached-copy-'))/'project')
    Path(project).mkdir()
    for name in ['bridge.py','rule.json','README.md']:
        shutil.copyfile(Path(workspace)/name, Path(project)/name)
    definition['coached_copy_source'] = workspace
    definition['copied_bytes_unchanged'] = True
definition['request'].update(project=project, prompt=Path(args.feedback).read_text(encoding='utf-8'))
if args.fresh_context:
    definition['request'].pop('resumeSessionId', None)
else:
    definition['request']['resumeSessionId'] = session
definition['artifacts_before']=[{'path':str(p),'sha256':digest(p)} for p in Path(workspace).glob('*') if p.name in ['bridge.py','rule.json','README.md']]
save_new(args.output,[definition])
print(json.dumps({'coached_request':str(args.output),'based_on_task':previous['task_id'],'business_source_written_by_host':False}))
