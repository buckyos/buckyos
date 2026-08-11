# kevent / kmsg DV test

This test module validates the real gateway-facing smoke path for `kevent` and
`kmsg` in a BuckyOS devtest environment.

Run from the repository root:

```bash
uv run src/check.py
uv run test/run.py -p kevent_kmsg/dv
```

Optional environment variables:

- `BUCKYOS_TEST_ZONE_HOST`: zone host, defaults to `test.buckyos.io`.
- `BUCKYOS_GATEWAY_BASE_URL`: gateway base URL, defaults to
  `https://${BUCKYOS_TEST_ZONE_HOST}`.
- `BUCKYOS_TEST_APP_ID`: test app id, defaults to `buckycli`.

The test keeps its scope intentionally small:

- `kmsg` CRUD and reconnect persistence are verified through the public service
  RPC client.
- `kevent` stream and publish are verified through `/kapi/kevent/stream` and
  `/kapi/kevent/publish`.
- The combined path verifies that `kevent` only acts as the notification signal
  while the durable payload is fetched from `kmsg`.
