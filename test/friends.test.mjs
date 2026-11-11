/*
 * Tests for the friends list's logic.
 *
 * Run with: npm test
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  cameOnline,
  canJoin,
  createEventQueue,
  friendActivity,
  noticesAllowed,
  onlineCount,
  ownAvailability,
  requestName,
  sortFriends,
  stillValid,
} from "../src/friends.js";

const playing = (party) => ({ status: "playing", title_id: "4D5308AB", party });

test("favourites come first, then who plays, who is online and who is not, then by name", () => {
  const friends = [
    { name: "Zed#1", presence: { status: "offline" } },
    { name: "Ana#2", presence: { status: "online" } },
    { name: "Bea#3", presence: playing("invite_only") },
    { name: "Cris#4", presence: { status: "offline" }, favourite: true },
    { name: "Abe#5", presence: playing("joinable") },
  ];
  assert.deepEqual(sortFriends(friends).map((f) => f.name), ["Cris#4", "Abe#5", "Bea#3", "Ana#2", "Zed#1"]);
  assert.equal(onlineCount(friends), 3);
});

test("what a friend is doing", () => {
  assert.deepEqual(friendActivity({ presence: playing("joinable") }), ["playingOpen", {}]);
  assert.deepEqual(friendActivity({ presence: playing("invite_only") }), ["playing", {}]);
  assert.deepEqual(friendActivity({ presence: { status: "online" } }), ["online", {}]);
  assert.deepEqual(friendActivity({ presence: { status: "offline", last_seen: "2026-11-01T10:00:00Z" } }), [
    "lastSeen",
    { at: Date.parse("2026-11-01T10:00:00Z") },
  ]);
  assert.deepEqual(friendActivity({ presence: { status: "offline", last_seen: null } }), ["offline", {}]);
});

test("only an open room can be joined, and not with the game's online play off", () => {
  assert.equal(canJoin({ presence: playing("joinable") }), true);
  assert.equal(canJoin({ presence: playing("invite_only") }), false);
  assert.equal(canJoin({ presence: { status: "online" } }), false);
  // Maintenance only stops new rooms: joining an open one still works.
  assert.equal(canJoin({ presence: playing("joinable") }, { kind: "maintenance", scope: "all", until: null }), true);
  assert.equal(canJoin({ presence: playing("joinable") }, { kind: "gameOff" }), false);
  assert.equal(canJoin({ presence: playing("joinable") }, { kind: "down" }), false);
});

test("a request names Gamertag#tag", () => {
  assert.equal(requestName(" Nimbus#4827 "), "Nimbus#4827");
  assert.equal(requestName("Kabosu#0117"), "Kabosu#117");
  assert.equal(requestName("Two Words#42"), "Two Words#42");
  assert.equal(requestName("Nimbus"), null);
  assert.equal(requestName("#12"), null);
  assert.equal(requestName("A name far too long#1"), null);
});

test("events wait while the launcher is not in front, and come back in order", () => {
  let front = false;
  const ran = [];
  const queue = createEventQueue(() => front);
  queue.offer({ event: "friend_request", data: { id: 1 } }, (e) => ran.push(e));
  queue.offer({ event: "invitation", data: { id: "a" } }, (e) => ran.push(e));
  assert.equal(ran.length, 0);
  assert.equal(queue.size, 2);
  assert.deepEqual(queue.take(), []);
  front = true;
  assert.deepEqual(queue.take().map((e) => e.event), ["friend_request", "invitation"]);
  assert.equal(queue.size, 0);
  queue.offer({ event: "friends", data: {} }, (e) => ran.push(e));
  assert.deepEqual(ran.map((e) => e.event), ["friends"]);
});

test("a waited event is only shown if it still holds", () => {
  const lists = { incoming: [{ id: 1, name: "Ana#1" }], invitations: [{ id: "a" }] };
  assert.equal(stillValid({ event: "friend_request", data: { id: 1 } }, lists), true);
  assert.equal(stillValid({ event: "friend_request", data: { id: 2 } }, lists), false);
  assert.equal(stillValid({ event: "invitation", data: { id: "a" } }, lists), true);
  // The invitation's room closed while the game ran: it is gone from the lists.
  assert.equal(stillValid({ event: "invitation", data: { id: "b" } }, lists), false);
  assert.equal(stillValid({ event: "presence", data: { id: 1 } }, lists), false);
});

test("a busy friend shows busy, online or playing", () => {
  assert.deepEqual(friendActivity({ presence: { status: "online", busy: true } }), ["busy", {}]);
  assert.deepEqual(friendActivity({ presence: { ...playing("joinable"), busy: true } }), ["playingBusy", {}]);
});

test("notices pop up only while the player shows themselves online", () => {
  assert.equal(ownAvailability(null), "online");
  assert.equal(noticesAllowed({ me: { availability: "online" } }), true);
  assert.equal(noticesAllowed({}), true);
  assert.equal(noticesAllowed({ me: { availability: "busy" } }), false);
  assert.equal(noticesAllowed({ me: { availability: "invisible" } }), false);
});

test("a friend comes online when they were offline and now are online or playing", () => {
  assert.equal(cameOnline({ status: "offline" }, { status: "online" }), true);
  assert.equal(cameOnline(undefined, playing("joinable")), true);
  assert.equal(cameOnline({ status: "online" }, playing("joinable")), false);
  assert.equal(cameOnline({ status: "online" }, { status: "offline" }), false);
});
