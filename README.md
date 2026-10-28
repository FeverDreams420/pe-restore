# PE Restore

Turns a crash report from the Parasite Eve PC port into a save on memory card 1,
so you can pick up right where the game crashed.

## How to use it

1. Close the game.
2. Open `pe-restore.exe`.
3. Click **Choose folder…** and pick your game folder (the one with the game's exe).
   The program remembers it for next time.
4. Pick a crash report from the list. The newest one is picked for you.
5. Optional: tick **Make Aya strong for testing** (level 99, 999 HP, a strong gun and
   armour, 6 Full Recovers).
6. Click **Write to slot 1**.
7. Start the game and load slot 1 on memory card 1.

You can also drag the game folder or a crash report onto the window (or onto
`pe-restore.exe`).

## Good to know

- If slot 1 already has a save, it asks first. The old save is moved to a
  `pe-restore-backups` folder next to `bu00`. Nothing is ever deleted.
- Newer builds of the port keep a full copy of the save each time a room starts,
  and the crash report holds that copy. The save you get puts you at the start of
  the room where the game crashed, with everything as it was when you walked in.
  Reports from a bit older builds hold the save from the moment of the crash instead.
- Crash reports from much older builds of the port don't have the full save in them
  and can't be used. Make a new one with a newer build.
- Mods: the save keeps the mods' data (Junk, Mod Catalog, key item pocket and so on)
  from the crash report. If the report doesn't have it, the mods' data of the save
  already in slot 1 is kept, and the program warns you that it's from that save,
  not from the crash.
- If the game doesn't show the save, newer builds may be reading the memory card in
  `%APPDATA%\Parasite Eve\memory-cards\bu00` instead. The program tells you when that
  is the case.

## Building

Needs Rust and the port's save editor at `../ParasiteEveNative/fork-pr-crash/save-editor`
(next to this folder).

```
cargo build --release
```

The program ends up in `target\release\pe-restore.exe`.
