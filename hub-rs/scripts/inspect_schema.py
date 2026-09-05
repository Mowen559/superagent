import sqlite3

con = sqlite3.connect(r"C:\Users\24778\.cache\codebase-memory-mcp\D-mowen-deepcloud.db")
cur = con.cursor()

print("=== Tables ===")
for (name,) in cur.execute("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"):
    print(name)

print("\n=== Schemas ===")
for (sql,) in cur.execute("SELECT sql FROM sqlite_master WHERE type='table' AND sql IS NOT NULL"):
    print(sql)
    print()

print("=== Row counts ===")
for (name,) in cur.execute("SELECT name FROM sqlite_master WHERE type='table'"):
    try:
        cnt = cur.execute(f"SELECT COUNT(*) FROM {name}").fetchone()[0]
        print(f"{name}: {cnt}")
    except Exception as e:
        print(f"{name}: error {e}")
