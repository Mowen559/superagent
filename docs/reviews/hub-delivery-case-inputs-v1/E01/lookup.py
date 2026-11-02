import argparse,pathlib,json
p=argparse.ArgumentParser();p.add_argument('--source',required=True);a=p.parse_args();print(json.dumps({'value':pathlib.Path(a.source).read_text()}))
