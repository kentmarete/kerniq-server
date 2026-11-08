#!/bin/sh
# Steady synthetic traffic for the demo dashboards. Whichever of $TARGETS
# resolves (the demo container of the active compose profile) receives it.
set -u

pick_target() {
  for t in $TARGETS; do
    if curl -s -o /dev/null --max-time 1 "http://$t/health"; then
      echo "$t"
      return 0
    fi
  done
  return 1
}

while :; do
  target=$(pick_target) && break
  sleep 2
done
echo "loadgen: sending traffic to $target"

i=0
while :; do
  i=$((i + 1))
  base="http://$target"
  curl -s -o /dev/null -X POST "$base/payments?type=transfer&amount_minor=$((i % 50000 + 100))&currency=USD"
  curl -s -o /dev/null -X POST "$base/payments?type=deposit&amount_minor=2500&currency=EUR"
  curl -s -o /dev/null "$base/accounts/$((i % 100))"
  [ $((i % 5)) -eq 0 ] && curl -s -o /dev/null -X POST "$base/loans?product=mortgage&amount_minor=$((i * 10000))"
  [ $((i % 3)) -eq 0 ] && curl -s -o /dev/null -X POST "$base/login?username=user$((i % 7))&password=correct-horse"
  [ $((i % 11)) -eq 0 ] && curl -s -o /dev/null -X POST "$base/login?username=mallory&password=x"
  [ $((i % 13)) -eq 0 ] && curl -s -o /dev/null -X POST "$base/payments?type=withdrawal&amount_minor=99999999&currency=GBP"
  # Streams: some complete, some are abandoned mid-generation.
  [ $((i % 4)) -eq 0 ] && curl -s -N -o /dev/null --max-time 10 "$base/v1/generate?model=chat-small&tokens=40" &
  [ $((i % 9)) -eq 0 ] && curl -s -N -o /dev/null --max-time 1 "$base/v1/generate?model=chat-large&tokens=200" &
  sleep 0.2
done
