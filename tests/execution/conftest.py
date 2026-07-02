"""
Execution test conftest.

Some tests (e.g. test_demo_unknown_pair_raises) register an httpx mock but the
code raises before making an HTTP call. We configure httpx_mock per-test via
the marker so unused mocks don't cause teardown failures.
"""
