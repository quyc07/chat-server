# chat-server

A self-hosted chat server and its TUI client. The server owns identity, relationships, and storage; the client reads and reacts to them.

## Language

**Conversation (会话)**:
A single thread of chat, either private (between the logged-in user and one friend) or group (between the logged-in user and one group). The unit the conversation list enumerates and the client navigates.
_Avoid_: chat, channel, room, thread
