import argparse
import datetime
import json
import math
from pathlib import Path
import sqlite3


def valid_number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) and value >= 0


def valid_cost(value):
    return isinstance(value, dict) and valid_number(value.get('amount')) and isinstance(value.get('currency'), str) and len(value['currency'].strip()) == 3 and value['currency'].strip().isascii() and value['currency'].strip().isalpha()


def recover(usage, record, declared_currency=None):
    usage = dict(usage)
    binding = record.get('binding') or {}
    if valid_cost(usage.get('cost')):
        return usage, {**usage['cost'], 'currency': usage['cost']['currency'].strip().upper()}, 'preserved_usage_cost'
    currency = binding.get('reported_cost_currency') or declared_currency
    if currency and valid_number(usage.get('reported_cost')):
        cost = {'amount': usage['reported_cost'], 'currency': currency.strip().upper()}
        if valid_cost(cost):
            return usage, cost, 'provider_reported'
    if usage.get('cost_unknown_reason'):
        return usage, None, usage['cost_unknown_reason']
    pricing = binding.get('pricing') or {}
    basis = pricing.get('basis') or {}
    if basis.get('kind') == 'units':
        if basis.get('unit') == 'image' and usage.get('image_units') is None:
            images = [item for item in (record.get('output') or {}).get('artifacts', []) if str(item.get('mime', '')).startswith('image/')]
            if images:
                usage['image_units'] = len(images)
        names = {'image': 'image_units', 'request': 'request_units', 'character': 'characters', 'audio_second': 'audio_seconds', 'video_second': 'video_seconds', 'reference_image': 'reference_images', 'second': 'compute_seconds', 'megapixel': 'megapixels'}
        amount = 0.0
        for component in [basis, *basis.get('addons', [])]:
            rate = component.get('amount')
            if not valid_number(rate):
                return usage, None, 'invalid_frozen_rate'
            if rate == 0:
                continue
            units = usage.get(names.get(component.get('unit'), ''))
            if not valid_number(units):
                return usage, None, 'missing_usage:' + str(component.get('unit'))
            amount += rate * units
    elif basis.get('kind') == 'tokens':
        if basis.get('tiers'):
            return usage, None, 'tiered_frozen_price_requires_runtime_review'
        if not all(valid_number(usage.get(key)) for key in ['input_tokens', 'output_tokens']):
            return usage, None, 'missing_token_usage'
        read = usage.get('cache_read_input_tokens', 0)
        write = usage.get('cache_write_input_tokens', 0)
        hour = usage.get('cache_write_1h_input_tokens', 0)
        ai, ii, vi = [usage.get(key, 0) for key in ['audio_input_tokens', 'image_input_tokens', 'video_input_tokens']]
        ao, io, vo = [usage.get(key, 0) for key in ['audio_output_tokens', 'image_output_tokens', 'video_output_tokens']]
        counts = [read, write, hour, ai, ii, vi, ao, io, vo]
        if not all(valid_number(value) for value in counts) or (read > 0 and ai + ii + vi > 0):
            return usage, None, 'invalid_or_ambiguous_usage'
        text_in = usage['input_tokens'] - read - write - ai - ii - vi
        text_out = usage['output_tokens'] - ao - io - vo
        if min(text_in, text_out, write - hour) < 0 or usage.get('reasoning_tokens', 0) > usage['output_tokens'] or usage.get('total_tokens', usage['input_tokens'] + usage['output_tokens']) < usage['input_tokens'] + usage['output_tokens']:
            return usage, None, 'invalid_usage'
        dimensions = [('input_token', text_in), ('output_token', text_out), ('cache_input_token', read), ('cache_write_input_token', write-hour), ('cache_write_1h_input_token', hour), ('audio_input_token', ai), ('image_input_token', ii), ('video_input_token', vi), ('audio_output_token', ao), ('image_output_token', io), ('video_output_token', vo)]
        amount = 0.0
        for name, count in dimensions:
            if count == 0:
                continue
            if not valid_number(basis.get(name)):
                return usage, None, 'missing_rate:' + name
            amount += basis[name] * count
    else:
        return usage, None, 'missing_frozen_pricing_or_verified_currency'
    cost = {'amount': amount, 'currency': pricing.get('currency', '').strip().upper()}
    if not valid_cost(cost):
        return usage, None, 'invalid_cost'
    return usage, cost, 'historical_pinned_pricing'


def repair(database, apply=False, audit=None, reported_provider=None, reported_currency=None):
    path = Path(database).resolve()
    connection = sqlite3.connect(path.as_uri() + ('?mode=rw' if apply else '?mode=ro'), uri=True, timeout=10)
    connection.row_factory = sqlite3.Row
    rows = connection.execute('SELECT u.event_id,u.provider_instance_name,u.provider_model,u.usage_json,u.finance_snapshot_json,e.record_json FROM aicc_usage_event u LEFT JOIN aicc_execution_record e ON e.task_id=u.task_id AND e.tenant_id=u.tenant_id WHERE u.finance_valid=0').fetchall()
    changes, skipped = [], []
    for row in rows:
        old = json.loads(row['usage_json'])
        record = json.loads(row['record_json']) if row['record_json'] else {}
        usage, cost, source = recover(old, record, reported_currency if row['provider_instance_name'] == reported_provider else None)
        if cost is None:
            skipped.append({'event_id': row['event_id'], 'reason': source})
            continue
        usage.update(cost=cost, cost_source=source)
        usage.pop('cost_unknown_reason', None)
        changes.append({'event_id': row['event_id'], 'provider': row['provider_instance_name'], 'model': row['provider_model'], 'original_usage': old, 'original_usage_json': row['usage_json'], 'original_finance': row['finance_snapshot_json'], 'usage': usage, 'cost': cost, 'source': source})
    report = {'database': str(path), 'apply': apply, 'recovered': len(changes), 'skipped': skipped, 'changes': changes}
    if apply:
        if not audit:
            raise ValueError('--apply requires --audit')
        stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
        backup = Path(str(path) + '.before-cost-repair-' + stamp)
        with sqlite3.connect(backup) as destination:
            connection.backup(destination)
        report['backup'] = str(backup)
        with Path(audit).open('x', encoding='utf-8') as output:
            json.dump(report, output, ensure_ascii=False, indent=2)
        with connection:
            for change in changes:
                result = connection.execute('UPDATE aicc_usage_event SET usage_json=?,finance_snapshot_json=?,finance_amount=?,finance_currency=?,finance_valid=1 WHERE event_id=? AND finance_valid=0 AND usage_json=? AND finance_snapshot_json IS ?', (json.dumps(change['usage']), json.dumps(change['cost']), change['cost']['amount'], change['cost']['currency'], change['event_id'], change['original_usage_json'], change['original_finance']))
                if result.rowcount != 1:
                    raise RuntimeError('usage changed during repair; transaction rolled back')
        report['applied'] = len(changes)
        Path(audit).write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    connection.close()
    return report


def main():
    parser = argparse.ArgumentParser(description='Dry-run or auditably repair unknown AICC ledger costs using frozen snapshots; never reads current model prices.')
    parser.add_argument('--database', required=True)
    parser.add_argument('--apply', action='store_true')
    parser.add_argument('--audit')
    parser.add_argument('--reported-provider')
    parser.add_argument('--reported-currency')
    args = parser.parse_args()
    if bool(args.reported_provider) != bool(args.reported_currency):
        parser.error('--reported-provider and --reported-currency must be supplied together after verifying the provider currency contract')
    if args.reported_currency and (len(args.reported_currency) != 3 or not args.reported_currency.isascii() or not args.reported_currency.isalpha()):
        parser.error('reported currency must be a three-letter code')
    report = repair(args.database, args.apply, args.audit, args.reported_provider, args.reported_currency)
    print(json.dumps({key: value for key, value in report.items() if key != 'changes'}, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
