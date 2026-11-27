# Run SQL

Open the **SQL** tab of a [connected](./connect.md) database to write and run SQL.

## Write and run a query

1. Type your SQL in the editor. Press **Ctrl+Space** for SQL keyword completion.
2. Choose **Run query**, or press **Cmd+Enter** (Ctrl+Enter).
3. Read the results below the editor.

```sql
SELECT id, email FROM users WHERE id = 1 ORDER BY id LIMIT 10;
```

- **+** (New query) opens another query tab. Query tabs belong to the connection they were opened on.
- **Query history** lists queries you ran on this connection; select one to put it back in the editor.
- **Save SQL** saves the editor's text as a `.sql` file.
- To make the editor taller or shorter, drag the bar under it. You can also select the bar with **Tab** and press **Up** or **Down**, or press **Alt+Up** or **Alt+Down** in the editor.

## Read-only connections

On a [read-only connection](./connect.md#read-only-or-read-write) you can run **one `SELECT` query at a time**. The database itself enforces read-only access. StoneQL accepts a deliberately narrow form of `SELECT`, so some valid queries are refused with “This read-only SQL form is not supported.” Scripts, changes to data, `CALL` and transaction commands are refused.

Results show up to 1,000 rows and 1 MiB; a shortened result is labelled. A single value too large to show makes the query fail rather than being cut off silently.

## Read-write connections: scripts

On a read-write connection, **Run query** runs everything in the editor as a script, one statement at a time, and stops at the first statement that does not succeed.

- Scripts containing `DELETE`, `UPDATE`, `DROP`, `TRUNCATE`, `ALTER`, `REPLACE`, `RENAME`, `GRANT` or `REVOKE` ask for confirmation first. These confirmations are safeguards, not permissions: your database account decides what is allowed.
- If the connection is already running something, the new request is refused. Nothing is queued or sent.
- Each statement is marked **succeeded**, **failed**, **unknown** or **not run**. A failure names the kind of constraint and the SQLSTATE when the database reports them. The database's own error text is not shown, because it can contain row values.
- Every result set is kept: use the **Result N · statement M** tabs to switch between them. Results are limited to 1,000 rows and 1 MiB per result set and 4 MiB per run; a shortened result is labelled.

Some statements are refused in scripts:

- Transaction control (`BEGIN`, `COMMIT`, `ROLLBACK`, savepoints) and autocommit changes: use the transaction buttons instead.
- On MySQL and MariaDB: `PREPARE`, `EXECUTE`, `DEALLOCATE`, `CALL`, MariaDB `SET STATEMENT … FOR`, and `IF`, `CASE`, loops and labelled blocks outside a stored program.

## Transactions

Without a transaction, each successful statement is saved immediately (autocommit).

1. Choose **Begin transaction**. Scripts you run now are part of one transaction, as are row edits in Browse.
2. Choose **Commit** to keep the changes, or **Rollback** to discard them.

Good to know:

- After a statement fails or is interrupted, only **Rollback** is offered.
- On MySQL and MariaDB only these run inside a transaction: `SELECT`, `WITH`, `VALUES`, `TABLE`, `SHOW`, `DESCRIBE`, `EXPLAIN`, `INSERT`, `UPDATE`, `DELETE`, `REPLACE`, `USE`, `SET @variable` and `CREATE`/`DROP TEMPORARY TABLE`.
- Structure changes and database switches are refused while a transaction is open.
- A transaction ends automatically after **60 seconds idle** or **15 minutes open**, or when the connection closes. StoneQL then tries to roll it back and tells you the result on your next action.
- **Commit** and **Rollback** are reported as done only when the server confirms them.
- **Disconnect** with an open transaction asks first, then rolls it back. Quitting StoneQL while a transaction is open or a statement is running also asks first; afterwards, check the server.

## Cancel a running statement

Choose **Cancel**. StoneQL asks the server to stop the statement and tells you whether the stop was confirmed. On PostgreSQL a statement that runs too long is stopped by the server and shown as **failed** (SQLSTATE 57014).

## Unknown outcomes

Sometimes StoneQL cannot know whether the server applied a statement, for example when it was cancelled after being sent, the connection was lost, or a Commit was not confirmed in time. StoneQL then marks it **unknown**. It never reports such work as succeeded and never runs it again.

1. Every unknown outcome is listed in a red panel above the workspaces headed **Outcome unknown — the server may or may not have applied this work.**, with the database it concerns.
2. Check the server yourself (for example, query the rows you changed).
3. Choose **Acknowledge** to hide the entry. Acknowledging does not mark the work as done or undone.

The list survives reloading, quitting and crashes. While 16 unknown outcomes are unacknowledged, StoneQL starts no new read-write work (an open transaction can still be committed or rolled back). If StoneQL cannot save this list on your computer, it says so and refuses new read-write work until it can.

## For developers

[Read-write SQL execution](../development/sql-execution.md) and [read-only SQL safety](../development/read-only-safety.md) give the full behaviour and limits.
