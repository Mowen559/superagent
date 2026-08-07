import re

def dump_functions(filepath):
    with open(filepath, 'r', encoding='utf-8') as f:
        content = f.read()
    
    # We want to extract functions named discover[A-Za-z0-9]+Models or Agents
    funcs = re.findall(r'func discover[A-Za-z0-9]+(?:Models|Agents)\(.*?\}\n\}?', content, re.DOTALL)
    
    with open("d:\\mowen\\deepcloud\\super-agent\\discover_funcs.go", "w", encoding='utf-8') as f:
        for fn in funcs:
            if "func discover" in fn:
                f.write(fn + "\n\n")

dump_functions("d:\\mowen\\deepcloud\\super-agent\\multica\\server\\pkg\\agent\\models.go")
print("Done")
