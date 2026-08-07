import sqlite3, os
c = sqlite3.connect(os.path.expanduser(r"~\.cache\codebase-memory-mcp\D-mowen-deepcloud.db"))
print("File node properties samples:")
for r in c.execute("SELECT file_path, properties FROM nodes WHERE label='File' LIMIT 5"):
    print("  ", r)
print("\nFunction node properties samples:")
for r in c.execute("SELECT name, properties FROM nodes WHERE label='Function' AND properties != '{}' LIMIT 5"):
    print("  ", r)
print("\ndistinct property keys on File nodes:")
for r in c.execute("SELECT DISTINCT json_each.key FROM nodes, json_each(nodes.properties) WHERE label='File' LIMIT 20"):
    print("  ", r)
