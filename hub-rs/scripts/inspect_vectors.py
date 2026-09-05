"""Probe CBM node_vectors / token_vectors BLOB format."""
import sqlite3
import struct
import os

DB = os.path.expanduser(r"~\.cache\codebase-memory-mcp\D-mowen-deepcloud.db")
conn = sqlite3.connect(DB)
cur = conn.cursor()

for table in ("node_vectors", "token_vectors"):
    n = cur.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
    print(f"{table}: {n} rows")

print()
row = cur.execute(
    "SELECT nv.node_id, n.name, n.label, LENGTH(nv.vector) "
    "FROM node_vectors nv JOIN nodes n ON n.id = nv.node_id LIMIT 3"
).fetchall()
for node_id, name, label, blen in row:
    print(f"node_id={node_id} name={name} label={label} blob_len={blen}")

blob = cur.execute("SELECT vector FROM node_vectors LIMIT 1").fetchone()
if blob:
    b = blob[0]
    blen = len(b)
    print(f"\nblob bytes: {blen}")
    # try float32
    if blen % 4 == 0:
        dim = blen // 4
        f32 = struct.unpack(f"<{dim}f", b)
        print(f"as float32: dim={dim}, first8={[round(x,4) for x in f32[:8]]}")
        norm = sum(x * x for x in f32) ** 0.5
        print(f"float32 L2 norm={norm:.4f}")
    # try int8
    i8 = struct.unpack(f"<{blen}b", b)
    print(f"as int8: dim={blen}, first16={list(i8[:16])}")
    norm8 = sum(x * x for x in i8) ** 0.5
    print(f"int8 L2 norm={norm8:.2f}")

tok = cur.execute("SELECT token, LENGTH(vector), idf FROM token_vectors LIMIT 5").fetchall()
print("\ntoken_vectors samples:")
for t, l, idf in tok:
    print(f"  token={t!r} blob_len={l} idf={idf}")

conn.close()
