# Browse and edit tables

These steps assume you are [connected](./connect.md). Editing needs a **read-write** connection.

## Find your way around

- The **Explorer** on the left lists databases and their tables and views. Select a table to open it in **Browse**. Drag the Explorer's edge to resize it.
- Each database row (arrow, icon and name) expands or collapses its table list. With the keyboard, focus it and press Enter or Space.
- The breadcrumbs at the top show the connection, database and table you are in.
- Each connection tab keeps its own navigation, SQL tabs and history.

## Switch databases (MySQL and MariaDB)

For MySQL and MariaDB the Explorer lists every database your account can see, with the connection's default database open first.

1. Open another database (or one of its tables) in the Explorer.
2. StoneQL switches the session to it on the same connection. Browse, SQL, edits, import and export now use that database. Unsaved query tabs are kept.

A switch is refused while a transaction is open. If the server refuses the database (no access, or it does not exist), StoneQL says so and stays on the current one.

PostgreSQL connections stay on the database they were configured with and show the tables of its current schema.

## Create a database (MySQL and MariaDB)

On a read-write connection:

1. Choose the **+** next to **DATABASES** in the Explorer.
2. Enter a name and, optionally, a character set and collation.
3. Review the SQL that StoneQL shows, then confirm.

## Browse rows

- Rows are read from the server one page at a time. Choose the page size and move between pages; the total row count is shown (if counting takes too long, the page is still shown and the total is marked unavailable).
- Select a column header to sort; use the filter to narrow the rows. Filter text containing a backslash is not accepted.
- Drag a column header's edge to resize it; double-click the edge to fit the content. Widths are remembered per table; reset them from the grid's context menu.
- Grids show exactly what the database stores:
  - SQL **NULL** appears as a muted italic *NULL*; the text `'NULL'` appears as ordinary text, and an empty string as an empty cell.
  - Tabs, line feeds and carriage returns appear as small ⇥, ↵ and ␍ markers (CRLF as one ␍↵) so rows stay one line high. The markers are only for display: editing, copying, sorting and filtering use the real value, and copying puts the real characters on the clipboard.
  - A cell's tooltip reads “SQL NULL” only for a real NULL. Screen readers announce “SQL NULL” and each marker as “tab”, “line feed”, “carriage return” or “CRLF”.
  - Large numbers (BIGINT, DECIMAL) and dates and times are shown exactly as stored.

## Edit rows

Row editing works on read-write connections for **tables with a primary key**. Tables without a primary key, views and binary columns are read-only.

- **Edit a value:** double-click a cell, change it and press Enter. Tick **NULL** to store SQL NULL, or **Default** to use the column's default, where the column allows it.
- **Edit a whole row:** use the row's **Edit** action.
- **Insert a row:** use the **Insert** tab.
- **Delete rows:** use the row's **Delete** action and confirm.

Each change is sent as one statement that identifies the row by its full primary key. If the row was changed or deleted on the server since you loaded it, StoneQL reports a conflict instead of saving. StoneQL asks before discarding unsaved changes.

If you have started a transaction in the [SQL tab](./run-sql.md#transactions), your edits are part of it and appear in Browse after **Commit**.

### Values with several lines

Editing a value that contains line breaks opens a larger editor over the grid, so row heights never change. The row form uses the same editor for TEXT columns and for any value with line breaks.

- Enter adds a new line; **Ctrl+Enter** or **Cmd+Enter** saves; Escape cancels (asking first if you changed something).
- The **Line endings** selector shows how line breaks will be saved:
  - Saving unchanged text writes the original value back exactly.
  - Edited text, including lines you type or paste, keeps the value's existing style (all LF, all CRLF or all CR).
  - A value with mixed endings starts on *Mixed (as stored)*: each break keeps its ending while the number of breaks is unchanged. If you add or remove breaks, choose LF, CRLF or CR before saving.
  - Choosing LF, CRLF or CR converts every line break when you save; this is the only way line endings change.
  - New line breaks in a value that had none are saved as LF unless you choose otherwise.

## Change table structure

Open a table and choose **Structure** to see its columns, indexes and foreign keys as the server reports them. On a read-write MySQL, MariaDB or PostgreSQL connection you can:

- **Add column**, **Change** a column (rename it or edit its definition) and **Drop** a column.
- **Add index** (unique or not, with the columns in the order you choose) and drop a secondary index. A primary key or an index that belongs to a constraint is not dropped on its own.

How it works:

1. Make your change in the form.
2. StoneQL shows the exact SQL it will run. Review it and choose **Apply**.
3. Dropping a column or changing a definition asks for confirmation (PROD connections always ask).

On MySQL and MariaDB, **Change** starts from the column's full definition, so collation, comment and other attributes stay unless you edit them. On PostgreSQL, Change edits the type (with an optional `USING` expression), whether NULL is allowed and the default, and alters only what you changed.

Your account needs the privileges to change the table (for example `ALTER` and `INDEX` on MySQL/MariaDB, or table ownership on PostgreSQL); otherwise the server refuses and nothing changes. Structure changes are refused while a transaction is open, because the server would commit it.

## Preferences

- The footer switches between the **Light** and **Dark** theme. StoneQL starts in Light whatever your system appearance; your choice is remembered.
- **Settings** sets the rows per page and shows keyboard shortcuts.

## Not available yet

Search, table Operations (rename, copy, truncate, drop), bulk Copy of selected rows, and managing views, routines, triggers and events are not available for live connections yet. See [What's in this release](./beta.md).
