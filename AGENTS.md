booru_viewer: fast native reference-image search over a read-only Danbooru mirror; not a web app

## invariants
- Durable booru index in XDG data; media bytes in XDG cache; mirror stays anonymous
- Authenticated mutations: optional, user-initiated capabilities; secret bytes stay outside config and durable state
