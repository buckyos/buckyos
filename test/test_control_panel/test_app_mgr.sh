#!/bin/bash
deno run --allow-net --allow-read --allow-env \
  --unsafely-ignore-certificate-errors \
  test_app_mgr.ts