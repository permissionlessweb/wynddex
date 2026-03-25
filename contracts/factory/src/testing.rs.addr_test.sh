#!/bin/bash
# Just check from what we know from the test output:
# left[0] contract = cosmwasm1k3356xt8xkpegg52zu909l6gq5tgyn7v7gwj6mjw0c55nqzmrctsp2ljgf  (actual first pair)
# left[1] contract = cosmwasm1f2kmm23y8vn2ykrnwur2w4hv9y4ydjlrwlqulx4fa5g7x7udgqasuvc528  (actual second pair)
# right[0] contract = cosmwasm1f2kmm23y8vn2ykrnwur2w4hv9y4ydjlrwlqulx4fa5g7x7udgqasuvc528  (expected first)  
# right[1] contract = cosmwasm1k3356xt8xkpegg52zu909l6gq5tgyn7v7gwj6mjw0c55nqzmrctsp2ljgf  (expected second)
echo "pair0000 addr = $(grep -m1 'pair0000' /tmp/addr_out.txt 2>/dev/null || echo 'unknown')"
