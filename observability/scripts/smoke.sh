#!/bin/sh
# Calls every demo endpoint once so that every metric family exists.
#   observability/scripts/smoke.sh [host:port]
set -eu
base="http://${1:-localhost:3000}"
curl -fs -o /dev/null -X POST "$base/payments?type=transfer&amount_minor=1250&currency=USD"
curl -fs -o /dev/null -X POST "$base/payments?type=deposit&amount_minor=2500&currency=EUR"
curl -s -o /dev/null -X POST "$base/payments?type=withdrawal&amount_minor=99999999&currency=GBP"
curl -fs -o /dev/null "$base/accounts/7"
curl -s -o /dev/null "$base/accounts/0"
curl -fs -o /dev/null -X POST "$base/loans?product=mortgage&amount_minor=1000"
curl -fs -o /dev/null -X POST "$base/login?username=alice&password=correct-horse"
curl -s -o /dev/null -X POST "$base/login?username=mallory&password=x"
curl -fsN -o /dev/null "$base/v1/generate?model=chat-small&tokens=10"
curl -sN -o /dev/null --max-time 0.3 "$base/v1/generate?model=chat-large&tokens=200" || true
sleep 0.5
echo "smoke traffic sent to $base"
