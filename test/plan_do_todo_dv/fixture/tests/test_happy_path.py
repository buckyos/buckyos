import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from service import ReservationService  # noqa: E402


class HappyPath(unittest.TestCase):
    def test_reserve_within_stock(self):
        s = ReservationService({"apple": 10})
        self.assertEqual(s.reserve("t1", "r1", "apple", 3), {"ok": True, "remaining": 7})
        self.assertEqual(s.stock("apple"), 7)

    def test_same_tenant_retry_is_applied_once(self):
        s = ReservationService({"apple": 10})
        first = s.reserve("t1", "r1", "apple", 4)
        again = s.reserve("t1", "r1", "apple", 4)
        self.assertEqual(first, again)
        self.assertEqual(s.stock("apple"), 6)


if __name__ == "__main__":
    unittest.main()
