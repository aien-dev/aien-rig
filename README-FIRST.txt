AIEN RIG - READ ME FIRST
========================

AIEN Rig is one program for your gaming PC (RTX 5070, Windows 11). It opens a
page in your web browser with three tabs:

  Play    - the FPS tuner: measures your games, makes only safe changes,
            keeps a receipt of every change, and can undo all of them.
  Create  - the YouTube helper: pick a finished video or caption file and get
            title ideas, a description, chapters and Shorts picks.
  Chat    - talk to our own AI engine running on your graphics card.

Everything runs on your PC. Nothing is uploaded anywhere.


GETTING IT
----------

1. Download this file (the newest version is always at this link):
   https://github.com/aien-dev/aien-rig/releases/latest/download/aien-rig-windows-x64.zip

2. Open your Downloads folder, right-click aien-rig-windows-x64.zip and pick
   "Extract All...", then "Extract".

3. Open the new folder, then the "AIEN Rig" folder inside it. You should see:
     aien-rig.exe       (the app)
     rig-tune.exe       (the FPS tuner, used by the Play tab)
     rig-create.exe     (the YouTube helper, used by the Create tab)
     rig-listen.exe     (speech to text for videos; only in some downloads)
     README-FIRST.txt   (this file)
     VERSION.txt        (which version you have)
     LICENSE.txt
   Keep these files together in one folder.

4. Double-click aien-rig.exe.
   Windows may show a blue box "Windows protected your PC". That is because
   the program is new and not signed yet. Click "More info", then "Run anyway".

5. A black window opens and your browser opens the AIEN Rig page.
   Keep the black window open while you use AIEN Rig. Closing it quits.
   If the browser does not open by itself, go to:  http://127.0.0.1:18100


USING IT
--------

Play tab
  - Scan: finds your games.
  - Pick a game, then Measure: records how it runs now. If it asks you to,
    start the game and play normally until it says it is done.
  - Apply: makes the safe changes for that game. A receipt is saved.
  - Undo everything: puts every change back the way it was.
  - Game mode: while one of the listed games runs, AIEN Rig pauses the AI
    engine (so the game gets all 12 GB of graphics memory) and makes sure
    Windows Game Mode is on. When the game closes it puts both back.
    You can change the list of games on the Settings tab.

Create tab
  - Easiest today: a caption file. In YouTube Studio open your video,
    Subtitles, and download the .srt or .vtt. Then click
    "Pick a caption file..." (the file window can open behind the browser;
    check the taskbar), or paste the full path.
  - A video file works only if rig-listen.exe is in the folder.
  - Click "Make it". Each result has a Copy button. All files (including
    youtube.txt with everything in order) are saved too; the page says where.
  - The Create tab uses the AI engine, so it needs the engine running (see
    Chat below). Until then it can still clean filler words out of captions.

Chat tab
  - Click "Start engine", wait, then type.
  - The engine runs inside WSL2 (Windows' built-in Linux). See "Not tested
    yet" below: chat is not expected to work on your PC yet.


WHAT WORKS TODAY
----------------

  - The app itself: the page, the three tabs, settings, receipts, the game
    watcher. It is built and its tests run on Windows by GitHub's build
    machines every time we change it.
  - The Play tab works when rig-tune.exe is in the folder, and the Create
    tab when rig-create.exe is. VERSION.txt lists which programs this
    download contains.


NOT TESTED YET
--------------

  - Nothing has run on your PC or on any RTX 5070 yet. You are the first.
  - Chat: our engine reaches the graphics card through WSL2, and that path is
    still being built. Starting the engine will most likely fail for now.
    That is expected; the page will say so.
  - Game mode switching has not been tried with real games yet.
  - The file picker on the Create tab has not been tried on Windows 11 yet.


SENDING RESULTS BACK
--------------------

Every time Play or Create runs, AIEN Rig saves a receipt. To send them:

1. Press Windows key + R, paste this and press Enter:
       %LOCALAPPDATA%\AIEN-Rig
2. Right-click the "receipts" folder, pick "Compress to ZIP file".
3. Send that zip plus the VERSION.txt from the AIEN Rig folder to Drake.
4. If something went wrong, also send a photo or screenshot of the black
   window and of the page, and one sentence about what you clicked.

Or open an issue at https://github.com/aien-dev/aien-rig/issues and attach
the same files.


REMOVING IT
-----------

1. Play tab: click "Undo everything" first.
2. Close the black window.
3. Delete the AIEN Rig folder you extracted, and the folder
   %LOCALAPPDATA%\AIEN-Rig  (settings and receipts).

AIEN Rig is free software under the GNU AGPL-3.0 (see LICENSE.txt).
Source: https://github.com/aien-dev/aien-rig
