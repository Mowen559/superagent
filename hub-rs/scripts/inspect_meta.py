import sqlite3, os
c = sqlite3.connect(os.path.expanduser(r"~\.cache\codebase-memory-mcp\D-mowen-deepcloud.db"))
print("projects:", c.execute("SELECT name, root_path FROM projects").fetchall())
print("vector projects:", c.execute("SELECT DISTINCT project FROM node_vectors LIMIT 3").fetchall())
print("edge types:")
for r in c.execute("SELECT type, COUNT(*) c FROM edges GROUP BY type ORDER BY c DESC LIMIT 12"):
    print("  ", r)
print("node labels:")
for r in c.execute("SELECT label, COUNT(*) c FROM nodes GROUP BY label ORDER BY c DESC LIMIT 12"):
    print("  ", r)
