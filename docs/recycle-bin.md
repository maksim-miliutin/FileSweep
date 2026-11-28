# Moving copies to the Recycle Bin

Status: a plan. Nothing of it is built, and the scanner stays read-only.

The window will let you pick copies from groups of identical files and move
them to the Recycle Bin, so that you can still put any of them back. These are
the rules the code has to keep. Each one is checked in Rust, not only in the
window, and each one gets a test.

## Rules

1. **The Recycle Bin or nothing.** A file is moved to the Recycle Bin or left
   where it is. FileSweep never deletes a file for good, and never falls back
   to that when the Recycle Bin cannot take a file: a network drive, a drive
   with no Recycle Bin, a file larger than the Recycle Bin allows. Such a file
   stays and is reported.
2. **One copy always stays.** In every group at least one copy is kept, and it
   must still be there, unchanged, when the others are moved. A request that
   would leave a group with no copy is refused whole.
3. **Only what you chose, after you confirmed.** Nothing is moved because a
   scan found it. The confirmation says how many files and how many bytes will
   go, and nothing moves before it.
4. **The file is still the one that was scanned.** Right before a copy is
   moved, its size and time of change are read again and its content is hashed
   again, and so is the copy that stays. If anything differs, the file stays
   and is reported.
5. **The exact path.** A file is named by the path the scanner met, never by
   the text the window shows. The window sends numbers, as it already does to
   show a file in its folder.
6. **Copies that may be one file stay.** A group marked "may be one file on
   disk" (hard links) frees no space when one name goes, so none of its copies
   is offered.
7. **One file at a time.** Each move is its own operation, so a refusal or a
   failure touches that file only. A move in progress can be stopped between
   files.
8. **A record of what moved.** After a run, the window lists every file that
   moved and every file that stayed, with the reason.
9. **No elevation.** FileSweep runs without administrator rights and asks for
   none. A file it may not move stays.
10. **The writing code lives apart.** The only code that changes the disk is a
    small crate of its own. The scanner and the session keep their read-only
    rules: no unsafe code, no write calls.

## How Windows does it

The plan is `IFileOperation::DeleteItem` with `FOFX_RECYCLEONDELETE` and no
dialogs, one file per operation, with a progress sink. Before each delete
Windows calls the sink's `PreDeleteItem` with flags; if they lack
`TSF_DELETE_RECYCLE_IF_POSSIBLE`, the file would be destroyed rather than
recycled, so the sink returns an error and the file stays. An error there
cancels the operation, which is one more reason for one file per operation.

This has to be confirmed on a real computer before the code is trusted: on a
fixed drive, a USB stick, a network share and a file larger than the Recycle
Bin allows.

## Open questions

- Whether to offer picks such as "keep the oldest copy", always shown and
  confirmed before anything moves.
- Whether the last access times should stay as they were while files are read
  again for rule 4.
