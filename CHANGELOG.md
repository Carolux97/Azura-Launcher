# Azura Launcher changelog

What each launcher release brings, newest first, as players see it: the launcher shows its own
version's section once in Play after it is installed or updated, and the release scripts publish
it as the update's notes (`scripts/release-common.sh`). Bumping the version adds its section, and a
release without one fails. One list item (`- `) per change, in English; an item may continue on
indented lines.

## 0.6.9

- An edition of the game the launcher does not support yet is reported on Alboria's official
  Discord: Installation copies its details and Report on Discord opens the server.
- The launcher now comes from its own repository, and the part that builds the game is a separate
  kit; nothing changes in how you install, update or play.

## 0.6.8

- The game's log notes every audio dropout (a moment of silence because the game's audio came
  late), to find the cause of the short dropouts some players hear in cinematics on a Mac. If you
  hear one, Report a problem and say roughly when.

## 0.6.7

- Report a problem, in Installation, makes a .zip to attach to a report: the newest game logs, the
  settings and the system, cleaned of names, addresses and folders. You see the list before saving it.
- RAAM's Shadow: its achievements show their icons in the War Journal.

## 0.6.6

- Co-op campaign: "The host paused the game" goes away on the player who joins also when the host
  resumes while that player has no character yet (a checkpoint reload) or the chapter changes.
- Online play may send each player up to 100 KB a second, where the game allowed 10 KB, so busy
  scenes no longer hold back the other player's game.

## 0.6.5

- Co-op campaign: the player who joins gets their character before the chapter 6 zipline, also with a
  high ping, and can shoot on it; a weapon switch that got stuck on their side is cleared by itself.
- Co-op campaign: "The host paused the game" goes away as soon as the host resumes, also after the host
  reloads a checkpoint from its pause menu.
- Azura's own menus show every letter of their languages (Czech, Polish, Hungarian, Russian and the
  others), where some showed a question mark.
- PlayStation controllers: the L1 and R1 glyphs sit on the same line as the other buttons.

## 0.6.4

- The Video settings have a menu of their own, with the settings grouped and what each one does. What
  you choose is kept: a resolution or image scaling chosen by hand no longer goes back to what the
  upscaling preset picks, and the menu shows what the preset picks while it decides them. It also
  opens the brightness calibration and can put every Video setting back to its default.
- Alboria's notices show over the game's menus and ours, and their time left is a ring around
  Alboria's planet that no longer crosses the notice's frame.

## 0.6.3

- Public matches: you are sent to the closest host, measured without anyone seeing anyone's address,
  and never back into a match you just failed to join.
- Public matches: in the lobby, the teams are shared out again when someone joins or leaves, and the
  lobby's list shows each team right.
- Public matches go on much sooner when the host drops: the match moves to another player in about 9
  seconds instead of 16.
- Who hosts a public match no longer shows in the lobby's list or by which side they play.
- Alboria's notices and the countdowns to a restart or maintenance now show in the game too.
- The Versus, Horde and Beast online menus play their sounds.
- RAAM's Shadow: its chapters show in Select chapter when it is the only add-on you have.
- macOS: the game's memory no longer grows with each map or chapter you load.
- The launcher has Blog and Discord buttons in Account, and can approve a browser signing in to your
  account on alboria.org.

## 0.6.2

- RAAM's Shadow starts again when you play in an Alboria party: choosing it no longer starts the
  main campaign.
- The campaign menus no longer offer public campaign matches, which Azura doesn't have.

## 0.6.1

- Czech and Hungarian: play the game in them, with Azura's own menus and the Alboria overlay too.
- The Latin American Spanish edition of the game is supported.
- Joining a friend works again when the host has more multiplayer map packs than you.
- The lobby only offers the maps everyone in it has, in the host's map list and in the vote. Joining
  a match on a map you don't have now tells you so.
- Leaving a friend's party takes you out of their room too.
- Friends: choose to show yourself online, busy or invisible, from the launcher or the overlay. You
  are told when a friend comes online, and friends lists update at once.
- The overlay: your status under your name (Y, or E on the keyboard), X copies your room's code,
  and its pages change with LB/RB only.
- Leaving a public match from the pause menu no longer says the room closed.
- macOS: the game's memory no longer grows the longer you play.
- Corrected translations in the game's menus.
- The launcher's release notes open in a window of their own, which scrolls, instead of stretching
  the Play page.

## 0.6.0

- Public matches on Alboria. Versus, Horde and Beast have an online row in their menus: choose the
  playlists you like and Alboria finds other players and puts you in a match, your party always on
  one team. Players can join a match in progress, taking a bot's place, and when the host leaves
  the match goes on with another player as its host. Ranked matches aren't available yet.
- Friends can join you in a public match while your team has a place, from the game's friends
  list, the launcher or an invitation. If your team is full the invitation still arrives, and
  joining tells them there is no place left.
- When a party's leader joins another party, their whole party comes along, as long as everyone
  fits.
- Leaving a public match takes you out of your party: the others play on. A player who does nothing
  for two minutes in a public match is removed from it, the host included.
- Your Alboria friends in the game: the game's own friends list shows who is online and playing,
  and you can join them or invite them from there. Xbox Live parties are on Alboria by default.
- The Alboria overlay inside the game (Back + RB on a controller): friends, invitations and your
  account without leaving the game. Invitations and friend requests show as notices over the game,
  and Back + RT accepts an invitation.
- The launcher has a new look, Alboria's, and a friends list on the Play page: see who is playing
  and join their room in one click. It also shows Alboria's notices and whether the service is up.
- When Alboria is in maintenance, the game says so in its own language.
- Choose the saves folder from the Saves page; your saves are backed up first.
- The update button asks before updating, and waits while the game is running.
- The multiplayer map packs are recognised, listed and can be removed in Downloadable content.
- If your edition of the game isn't supported yet, the launcher shows its details with a button to
  copy them for your report.
- Smoother frame pacing: the game's frames follow your display's refreshes on macOS, Linux and
  Windows.
- Sounds in Azura's own menus in the game and in the Alboria overlay.
- Playing online needs this version: older versions can no longer connect to Alboria.

## 0.5.1

- Friends can join your room on Alboria as soon as the game starts. In 0.5.0 the room the game
  opened at start-up could not be joined: whoever used its code was sent back after a few seconds,
  until the party was switched to Local and back.
- The party panel in the main menu names your room's type, Joinable or Invite only, and follows it
  when you change it, also for the friends in your room.
- The game signs in to Alboria about once an hour instead of each time you open the party options
  or change the party's type, so changing it several times in a row no longer runs into Alboria's
  limit of sign-ins.
- macOS at 1920 × 1080: glass, water and smoke no longer look distorted from some angles, and
  textures no longer turn into a pixelated green one.
- Linux and Windows: the game's window shows the Azura icon in the taskbar, and on Windows
  azura.exe has it too.

## 0.5.0

- macOS: a new, faster way to draw the game, now the default. The launcher no longer replays the
  console's graphics commands step by step: it follows what the game means to do, skipping copies
  of the picture that were not needed and writing finished images straight from the graphics card.
  It works at every resolution scale. On Linux and Windows the console's way stays the default.
- The game's own effects can be switched off in the Video options: depth of field, dynamic
  shadows, ambient occlusion, bloom and lens flares. Turning off depth of field and dynamic shadows
  saves the most graphics card time (measured on macOS); the other three change the look only.
  Ambient occlusion stays off by default on Linux, as before.
- 1920 × 1080 in the render resolutions of the launcher and the game's Video menu. On macOS it sits
  between 1280 × 720 and 2560 × 1440 in detail and graphics card time; on Linux and Windows the game
  is drawn at 2560 × 1440 and scaled down to the screen, so it takes the graphics card time of
  2560 × 1440.
- macOS: cheaper pixel shaders in textures that need no sign handling, about 10% less graphics
  card time per frame in the scene measured. The first start after updating compiles the shaders
  again, with the usual "Compiling shaders" notice.
- RAAM's Shadow can be played. Downloadable content is a new panel in the launcher: it lists every
  Gears of War 3 pack with its state, and installs the packages you add into your saves folder. The
  launcher prepares what RAAM's Shadow needs from your own game files, and its achievements show in
  the achievements section.
- Downloadable content: it works when the game is a disc image (`.iso`) and packages can be added
  inside a `.zip`.
- Each installation now has a player identity of its own, so two players no longer share the same
  one. It is kept in the saves folder.
- The controller menu's texts follow the game's language.
- Alboria accounts, for playing over the internet with friends in this game and in the ones that
  come after it. The first time the launcher opens, also after updating, it asks how you want to
  play: create an account, sign in to yours or play with a local account (Local and System Link, no
  internet). Creating an account asks you to confirm you are 18 or older. An account has no
  password: a 6-digit code sent to your email signs you in. The new Account section shows your
  identity as Gamertag#tag, the gamertag (moved here from Installation), your devices, and lets you
  download or delete your data.
- Play with friends. System Link works on a local network: campaign co-op, Horde, Beast and
  private Versus, choosing System Link in the party options. Over the internet, with an Alboria
  account, a Joinable or Invite only party is a room on Alboria: friends join it with its code
  (Manage/Invite, Join by code). Players connect directly when their routers allow it, and through
  Alboria's relay when they don't. Options has a new Connection section with your Gamertag#tag, the
  direct connection setting and the latency.
- While Alboria can't send emails yet, accounts are created without a code. Once codes are back
  you'll have 48 hours to verify your email from Account (or Play), or the account will be deleted.
- Upscaling preset in the Video menu: it picks the render resolution for your screen and turns on
  image scaling (AMD FSR 1; MetalFX on macOS).
- Mouse: native mouse look with no aim assist, and the sideways turn while roadie running is
  capped.
- Linux and Windows: faster Vulkan, and 3840 × 2160 no longer shows a black picture.
- The launcher no longer stutters while it's in front: its animated background now costs a
  fraction of what it did.

## 0.4.5

- Linux and Windows: higher and steadier frame rates, most of all in combat, with light glows
  still hidden behind walls. The game gets what the graphics card has counted as visible sooner,
  copying the game's memory no longer splits up the graphics card's work, and less time is spent
  waiting for both to be ready.
- Linux and Windows: lights no longer flicker in busy scenes: when a frame tested more objects for
  visibility than there was room to count, the rest were taken as visible.
- Linux: far fewer stutters. Watching the game's memory no longer reads a large system file on every
  write (on Linux 6.11 and newer; older kernels work as before).

## 0.4.4

- Linux and Windows: light glows no longer show through walls. The stall reduction of 0.4.3 is
  removed, with its Stutter reduction row in the Video options: it made the game treat hidden
  lights as visible. A fix for those stalls that keeps the lights right will come later.

## 0.4.3

- Installing the game runtime shows why it failed next to the Install button, instead of looking
  as if nothing happened, and Install waits until a compiler is installed.
- Linux and Windows: smoother play. The game no longer waits for the graphics card to count what
  is visible before going on, which caused frequent short stalls.
- Vulkan (Linux and Windows): at 2560 × 1440 and 3840 × 2160, dark areas and shadows no longer turn green.

## 0.4.2

- Steam Deck and SteamOS: the launcher now finds the compiler once it is installed. The command it
  showed put the compiler inside a container, where the launcher cannot use it; it now installs
  clang on SteamOS itself, and the launcher says where to run it, that the deck user needs a
  password, and that a SteamOS update removes the compiler, so the command is run again after one.
- Bazzite and other image-based systems no longer suggest a container either.
- Video options: Field of view, from 80 (the game's) to 100 degrees. Aiming, cover and roadie run
  widen or narrow with it, and the camera turns as fast as with the game's field of view, with the
  mouse and the controller.
- Video options: Film grain can be turned off.
- With the mouse, aiming turns the camera as far as looking around does. Zooming in still turns it
  slower, and the keyboard and mouse settings' aim and zoom sensitivities still apply.

## 0.4.1

- A custom controller layout changes what the buttons do in play only: the menus keep the
  controller's own buttons (A accepts, B goes back), and their prompts show them.
- The PlayStation controllers in the controls menu are drawn closer to the real ones: the
  touchpad's light inside its edges, Create and Options beside it, and the PS button.
- Keyboard and mouse button prompts are easier to read: letters and digits in a smooth font, no
  glow smudging the text around them, and the movement keys drawn as two keys, W above ASD, instead
  of four tiny ones.
- PlayStation prompts: grey symbols for a DualSense and coloured ones for a DualShock 4, and the
  Create (Share) button where the game shows Back.
- Prompts in the HUD no longer cut the game's own marks around a button, such as the dots of a
  "hold" prompt.
- The button prompts take far less processor time: they no longer search the game's memory every
  two seconds, but look at each texture as the game loads it.
- Windows: the game no longer closes now and then as the intro movie starts.
- Windows: the game is no longer held at 20 FPS. It now reaches its frame rate limit, without
  tearing.

## 0.4.0

- Play has a Prepare shaders card: it compiles the shaders the game stored in your earlier
  sessions before you play, so they are ready instead of being made while you play. It shows its
  progress, remembers when it last ran, and offers to prepare again once the game has stored new
  shaders since.
- The launcher leaves the game the processor: its animations pause while the game runs or the
  launcher is in the background, it minimizes itself while you play and comes back when the game
  ends (Graphics, Launcher while playing), and it draws the game log only when it can be seen.
- The mouse turns the camera directly, as in a PC game: no dead zone for slow aiming, no limit on
  fast turns, and the camera stops as soon as the mouse does. Mouse sensitivity now sets the turn
  per pixel; if you raised it before, lower it again.
- Controls: in the game's Options, Controller and Keyboard and mouse open menus of their own.
  Controller offers the game's schemes (Default, Alternate) and a Custom one where any button can
  take another's action, shows what each does around a drawing of your controller, and has the
  game's controller settings (inverted axes, sensitivities, vibration and the rest). Keyboard and
  mouse gives every action two keys or mouse buttons (side buttons and the wheel included), and
  has the mouse settings: sensitivity for looking, aiming and zooming, inverted Y, acceleration,
  and pressing instead of holding to aim, zoom and roadie run.
- New keyboard and mouse defaults closer to Gears of War on PC, with no key doing two things.
  Keys you changed before are kept; Defaults in the Keyboard and mouse menu brings the new ones.
- In the menus Enter accepts and Esc goes back, as on a PC; in play Esc pauses, and on the title
  screen Enter starts. The main menu has an Options entry instead of Start's button.
- The button prompts in Azura's menus follow the device in your hands, keyboard and mouse or
  controller, and change as soon as you use another.
- Video options have a Performance overlay in the corner you choose: FPS shows the game's frame
  rate and the time per frame, Detailed adds the game's CPU and GPU use, and Advanced adds its
  memory use, a graph of the last seconds of frame times, the average frame rate and the 1% low.
- On Linux, the game and the programs the launcher starts use your system's libraries instead of
  the launcher's own copies (it fixes keyboard errors in the game log on newer systems).
- Windows (10 and 11, x86-64): the launcher comes as an installer, builds the game from your copy
  and keeps itself and the game up to date, as on macOS and Linux. To build the game it installs
  LLVM and Visual Studio's Build Tools with winget when they are missing.

## 0.3.0

- Local co-op: two players can play the campaign in split screen on one computer. In the game's
  Options, Local co-op gives a controller, or the keyboard and mouse, to player 2, who plays with
  a profile of their own (your gamertag with "(1)") and saves of their own.
- When a player's only controller switches off, the game pauses for them and a notice asks to
  reconnect it, or to press the confirm button on another controller to use that one. The same
  controller coming back plays for the same player again.
- On the title screen after player 2 joins, only player 1 can start, so the game always loads
  player 1's profile and saves.
- In the Options, Keyboard and mouse is followed by Local co-op, and the mature content settings
  (violence, mature language) are now at the end of Game.
- Image scaling offers what your graphics support: on Linux, AMD CAS and AMD FSR 1 as well.
- On Linux, far fewer hitches while the game prepares new graphics in the background.

## 0.2.2

- On macOS, Play shows the chip as one entry again, without an empty CPU entry.

## 0.2.1

- Fewer hitches the first time a new area or cutscene appears: the game prepares new shaders in
  the background, and shadows wait less for theirs.
- After this update, the game prepares its shaders again at its first start (the "Compiling
  shaders" notice); it takes a few minutes, only once.
- Play names the graphics as they are: Metal on macOS, and on Linux the processor and the graphics
  card as separate entries.

## 0.2.0

- Builds the game for this computer from your own copy of Gears of War 3, a folder or a disc
  image (.iso), with every fix of the project, and installs the compiler it needs when it is
  missing.
- Updates itself and the game when a new version is published: most game fixes install in seconds.
- Can be used with a controller (Xbox, PlayStation, Nintendo and others), with the buttons of the
  controller in hand on screen and an on-screen keyboard to type.
- In the game, the controller in your hands is player 1, whatever order your controllers were
  connected in.
- Display mode, render resolution, game language and gamertag, saves you can move and back up, and
  the achievements with their artwork.
