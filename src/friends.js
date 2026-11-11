/*
 * The friends list's logic (alboria-notes:azura/alboria-relay.md, section 4.7), apart from the page
 * so it can be tested on its own: the order of the list, what each friend is doing, whether their
 * room can be joined, a request's name, and the queue that holds the service's events while the
 * launcher is not in front.
 */

/** The order of presence in the list: playing first, then online, then offline. */
const PRESENCE_ORDER = { playing: 0, online: 1, offline: 2 };

/** Favourites first, then by what each friend is doing, then by name. */
export const sortFriends = (friends) =>
  [...friends].sort(
    (a, b) =>
      Number(Boolean(b.favourite)) - Number(Boolean(a.favourite)) ||
      (PRESENCE_ORDER[a.presence?.status] ?? 2) - (PRESENCE_ORDER[b.presence?.status] ?? 2) ||
      a.name.localeCompare(b.name),
  );

/** How many friends are online or playing. */
export const onlineCount = (friends) => friends.filter((friend) => friend.presence?.status && friend.presence.status !== "offline").length;

/**
 * What a friend is doing, as a locale key under `friends` and its values: playing (with whether
 * the room is open to join), online, busy (online or playing), or offline with when they were last
 * seen.
 */
export const friendActivity = (friend) => {
  const presence = friend.presence ?? { status: "offline" };
  if (presence.status === "playing" && presence.busy) return ["playingBusy", {}];
  if (presence.status === "playing") return presence.party === "joinable" ? ["playingOpen", {}] : ["playing", {}];
  if (presence.status === "online") return presence.busy ? ["busy", {}] : ["online", {}];
  return presence.last_seen ? ["lastSeen", { at: Date.parse(presence.last_seen) }] : ["offline", {}];
};

/** What the player chose to show their friends (`me` of the lists): online, busy or invisible. */
export const ownAvailability = (lists) => lists?.me?.availability ?? "online";

/** Whether notices may pop up: only while the player shows themselves online, not busy or invisible. */
export const noticesAllowed = (lists) => ownAvailability(lists) === "online";

/** Whether a friend's presence going from `before` to `after` is them coming online (or into a game). */
export const cameOnline = (before, after) =>
  (before?.status ?? "offline") === "offline" && Boolean(after?.status) && after.status !== "offline";

/**
 * Whether a friend's room can be joined from the launcher: they play with a joinable room, and the
 * game's online play is on (`block`, from serviceView). Maintenance only stops new rooms.
 */
export const canJoin = (friend, block = null) =>
  friend.presence?.status === "playing" && friend.presence.party === "joinable" && block?.kind !== "gameOff" && block?.kind !== "down";

/** The `Gamertag#tag` typed to send a request, tidied up, or null when it is not one. */
export const requestName = (typed) => {
  const match = /^(.{1,15})#(\d{1,4})$/.exec(String(typed ?? "").trim());
  if (!match || !match[1].trim()) return null;
  return `${match[1].trim()}#${Number(match[2])}`;
};

/**
 * Holds the service's events while the launcher is not in front (the game starting or running, the
 * window in the background or hidden) and gives them back, in order, once it is: they run then, and
 * each is checked against what is true by that time (stillValid).
 */
export const createEventQueue = (inFront) => {
  const waiting = [];
  return {
    /** Runs `event` with `run` now when the launcher is in front, or keeps it for later. */
    offer(event, run) {
      if (inFront()) run(event);
      else waiting.push(event);
    },
    /** The events that waited, in order, and an empty queue; none while still not in front. */
    take() {
      if (!inFront()) return [];
      return waiting.splice(0, waiting.length);
    },
    get size() {
      return waiting.length;
    },
  };
};

/**
 * Whether an event that waited still means something with the lists as they are now: a request
 * still waiting for an answer, an invitation still open. Other events only asked to read the lists
 * again, which the flush does anyway.
 */
export const stillValid = (event, lists) => {
  if (event.event === "friend_request") return lists.incoming.some((entry) => entry.id === event.data?.id);
  if (event.event === "invitation") return lists.invitations.some((entry) => entry.id === event.data?.id);
  return false;
};
