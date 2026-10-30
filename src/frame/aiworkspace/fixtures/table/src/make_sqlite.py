"""SQLite fixture for the table importer (表格数据导入 TODO T5, TI-10): three tables and a view; INTEGER, REAL,
NUMERIC(10,2), TEXT dates, BOOLEAN, DATETIME, BLOB, untyped and NULL columns; an integer above 2^53.

Run from this folder: python3 make_sqlite.py   (writes ../sqlite/shop.sqlite)"""

import os
import sqlite3

path = '../sqlite/shop.sqlite'
if os.path.exists(path):
    os.remove(path)
db = sqlite3.connect(path)
db.executescript('''
CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT NOT NULL, price NUMERIC(10,2), weight REAL, active BOOLEAN, added DATE, photo BLOB);
CREATE TABLE orders (id INTEGER PRIMARY KEY, product_id INTEGER, qty INTEGER, placed DATETIME, status TEXT, big INTEGER, extra);
CREATE TABLE "odd ""name""" (k TEXT, v TEXT);
CREATE VIEW active_products AS SELECT id, name FROM products WHERE active = 1;
''')
db.executemany('INSERT INTO products VALUES (?, ?, ?, ?, ?, ?, ?)', [
    (1, 'Kettle', '29.90', 1.25, 1, '2026-01-15', b'\x89PNG'),
    (2, 'Toaster', '45.00', 2.5, 0, '2026-02-01', None),
    (3, 'Mug', '6.5', 0.3, 1, None, b'\x00\x01'),
])
db.executemany('INSERT INTO orders VALUES (?, ?, ?, ?, ?, ?, ?)', [
    (1, 1, 2, '2026-03-01 10:00:00', 'open', 9007199254740993, 'x'),
    (2, 3, 6, '2026-03-02 11:30:00', 'done', 12, None),
    (3, 2, 1, '2026-03-03 09:15:00', 'open', 13, 4.5),
    (4, 1, 1, '2026-03-04 16:45:00', 'done', 14, 'y'),
    (5, 3, 3, '2026-03-05 08:00:00', 'open', 15, None),
    (6, 2, 2, '2026-03-06 12:00:00', 'open', 16, None),
])
db.executemany('INSERT INTO "odd ""name""" VALUES (?, ?)', [('a', '1'), ('b', '2')])
db.commit()
db.close()
print('wrote', path)
