RUNTIME_APP="/opt/buckyos/bin/node-daemon/BuckyOS Runtime.app"

tccutil reset ScreenCapture com.buckyos.runtime
tccutil reset Accessibility com.buckyos.runtime
open -n "$RUNTIME_APP" --args --request_permissions
