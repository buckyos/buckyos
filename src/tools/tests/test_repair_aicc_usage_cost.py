import importlib.util
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('repair_aicc_usage_cost', Path(__file__).resolve().parents[1] / 'repair_aicc_usage_cost.py')
repair = importlib.util.module_from_spec(spec)
spec.loader.exec_module(repair)


class RepairCostTests(unittest.TestCase):
    def test_only_actual_images_are_recovered_from_frozen_prices(self):
        record = {'binding': {'pricing': {'currency': 'USD', 'basis': {'kind': 'units', 'unit': 'image', 'amount': 0.039}}}, 'output': {'artifacts': [{'mime': 'image/png'}, {'mime': 'audio/wav'}, {'mime': 'image/jpeg'}]}}
        usage, cost, source = repair.recover({}, record)
        self.assertEqual(usage['image_units'], 2)
        self.assertAlmostEqual(cost['amount'], 0.078)
        self.assertEqual(source, 'historical_pinned_pricing')
        record['output']['artifacts'] = []
        self.assertIsNone(repair.recover({}, record)[1])

    def test_currency_needs_explicit_evidence_and_reported_cost_wins(self):
        usage = {'reported_cost': 0.015216}
        self.assertIsNone(repair.recover(usage, {})[1])
        self.assertEqual(repair.recover(usage, {}, 'USD')[1], {'amount': 0.015216, 'currency': 'USD'})
        self.assertIsNone(repair.recover({'reported_cost': -1}, {}, 'USD')[1])

    def test_missing_multimodal_rate_and_inconsistent_tokens_remain_unknown(self):
        record = {'binding': {'pricing': {'currency': 'USD', 'basis': {'kind': 'tokens', 'input_token': 0.0000015, 'output_token': 0.000009}}}}
        self.assertIsNone(repair.recover({'input_tokens': 3053, 'output_tokens': 46903, 'image_input_tokens': 1100}, record)[1])
        self.assertIsNone(repair.recover({'input_tokens': 3, 'output_tokens': 2, 'cache_read_input_tokens': 5}, record)[1])

    def test_dry_run_apply_backup_audit_and_idempotence(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'usage.db'
            with sqlite3.connect(path) as c:
                c.executescript('CREATE TABLE aicc_usage_event(event_id TEXT,task_id TEXT,tenant_id TEXT,provider_instance_name TEXT,provider_model TEXT,usage_json TEXT,finance_snapshot_json TEXT,finance_amount REAL,finance_currency TEXT,finance_valid INTEGER); CREATE TABLE aicc_execution_record(task_id TEXT,tenant_id TEXT,record_json TEXT);')
                c.execute('INSERT INTO aicc_usage_event VALUES (?,?,?,?,?,?,NULL,NULL,NULL,0)', ('e','task','tenant','sn','kimi',json.dumps({'reported_cost':0.1})))
            result = repair.repair(path, reported_provider='sn', reported_currency='USD')
            self.assertEqual(result['recovered'], 1)
            with sqlite3.connect(path) as c:
                self.assertEqual(c.execute('SELECT finance_valid FROM aicc_usage_event').fetchone()[0], 0)
            audit = Path(folder) / 'audit.json'
            result = repair.repair(path, apply=True, audit=audit, reported_provider='sn', reported_currency='USD')
            self.assertTrue(Path(result['backup']).is_file())
            self.assertEqual(json.loads(audit.read_text())['applied'], 1)
            with sqlite3.connect(path) as c:
                self.assertEqual(c.execute('SELECT finance_amount,finance_currency,finance_valid FROM aicc_usage_event').fetchone(), (0.1,'USD',1))
            self.assertEqual(repair.repair(path, reported_provider='sn', reported_currency='USD')['recovered'], 0)


if __name__ == '__main__':
    unittest.main()
