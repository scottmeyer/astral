#!/usr/bin/env python3
"""Matched full-history, provider-inline, and Astral-scheduled benefit trials.

Reports observed usage and wall time, not verified charges. Gateway mode uses
existing authorized routing without reading CLI credentials. Python 3.11+.
"""
from __future__ import annotations

import argparse
from copy import copy
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import random
import sys
import time
from urllib.parse import urlsplit
import uuid

import api_trial as api
import codex_trial as common
from inline_trial import prune_inline_history


def digest(value):
    return hashlib.sha256(json.dumps(value,sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()).hexdigest()


def task(seed):
    rng=random.Random(seed)
    expected={'deployment_label':f'deploy-{rng.getrandbits(96):024x}',
              'region':f'zone-{rng.getrandbits(64):016x}','budget':23,
              'recovery_code':f'recover-{rng.getrandbits(96):024x}',
              'audit_code':f'audit-{rng.getrandbits(96):024x}'}
    initial={k:v for k,v in expected.items() if k!='audit_code'}
    initial['budget']=17
    filler=''.join(f'{i:04d} synthetic build check passed; no configuration changes.\n' for i in range(1800))
    fixtures={'config':json.dumps(initial)+'\n'+filler,
              'audit':json.dumps({'audit_code':expected['audit_code']})+'\n'+filler}
    prompts=[
        'Read the config fixture. Preserve all fields. Reply only READY without repeating any values.',
        'Correction: budget is now 23, replacing 17. Keep other fields. Use no tools; reply only READY.',
        'Mark dependency review complete. Keep all earlier configuration. Use no tools; reply only READY.',
        'Mark build validation complete. Keep all earlier configuration. Use no tools; reply only READY.',
        'Mark staging validation complete. Keep all earlier configuration. Use no tools; reply only READY.',
        'Read the audit fixture. Preserve audit_code, the earlier configuration, and corrected budget. Reply only READY without repeating any values.',
        'Mark audit review complete. Keep both fixtures and corrections. Use no tools; reply only READY.',
        'Mark release review complete. Keep both fixtures and corrections. Use no tools; reply only READY.',
        'Return only JSON with deployment_label, region, budget (integer), recovery_code, and audit_code. Use no tools.',
    ]
    return expected,fixtures,prompts


def run_arm(args, arm, seed, folder):
    expected,fixtures,prompts=task(seed)
    opts=copy(args)
    opts.compaction_backend='inline'
    opts.allow_compatible_compaction=True
    proxy=common.Proxy(opts,'rolling' if arm=='astral' else 'passthrough',folder)
    lane='astral-benefit-'+str(uuid.uuid4())
    body={'model':args.model,'instructions':f'Trial label: {lane}. Do not repeat it. Preserve all fixture fields and corrections. Before the final JSON question, reply only READY, never repeating field values. Use read_fixture only when asked.',
          'tools':[api.TOOL],'store':False,'stream':True,
          'reasoning':{'effort':'medium','context':'all_turns'},
          'include':['reasoning.encrypted_content'],'prompt_cache_key':lane}
    if args.cache_mode=='implicit': body['prompt_cache_options']={'mode':'implicit','ttl':'30m'}
    if arm=='provider': body['context_management']=[{'type':'compaction','compact_threshold':args.threshold}]
    report={'arm':arm,'seed':seed,'expected':expected,'calls':[],'turns':[],'passed':False}
    history=[]
    loaded=[]
    response_compactions=0
    before_restart=None
    expected_final_hash=None
    started=time.monotonic()
    try:
        proxy.start()
        for number,prompt in enumerate(prompts,1):
            if number==len(prompts):
                before_restart=common.snapshot(proxy.state)
                proxy.close()
                proxy.start()
            history.append({'role':'user','content':[{'type':'input_text','text':prompt}]})
            if number==len(prompts):
                effective=history
                if arm=='astral':
                    paths=list((proxy.state/'lanes').glob('*.json'))
                    if len(paths)!=1: raise RuntimeError('expected_one_astral_lane')
                    state=json.loads(paths[0].read_text())
                    effective=state['projection']+history[state['cut']:]
                    expected_final_hash=digest(effective)
                wire=json.dumps({**body,'input':effective})
                report['canaries_absent_from_final_upstream_input']=all(v not in wire for v in expected.values() if isinstance(v,str))
                report['final_effective_input_bytes']=len(json.dumps(effective).encode())
            for attempt in range(3):
                body['input']=history
                call_started=time.monotonic()
                response=api.call(args,proxy,body,lane,folder/f'turn-{number}-{attempt+1}.sse')
                output=response['output']
                count=sum(i.get('type')=='compaction' for i in output)
                response_compactions+=count
                report['calls'].append({'turn':number,'seconds':round(time.monotonic()-call_started,3),
                                        'client_input_bytes':len(json.dumps(history).encode()),
                                        'native_compactions':count,'output_types':[i.get('type') for i in output]})
                history.extend(output)
                if arm=='provider': history=prune_inline_history(history)
                calls=[i for i in output if i.get('type')=='function_call']
                if not calls:
                    text=''.join(c.get('text','') for i in output if i.get('type')=='message' for c in i.get('content',[]) if c.get('type')=='output_text')
                    turn={'turn':number,'text':text}
                    if number==len(prompts): turn['correct']=json.loads(text)==expected
                    elif text.strip()!='READY': raise RuntimeError('values_must_not_be_echoed_before_final_recall')
                    report['turns'].append(turn)
                    print(json.dumps({'arm':arm,'seed':seed,'turn':number,'native_compactions':response_compactions,'correct':turn.get('correct')}),flush=True)
                    break
                for item in calls:
                    name='config' if number==1 else 'audit' if number==6 else None
                    if name is None or item['name']!='read_fixture' or json.loads(item['arguments'])!={'name':name}:
                        raise RuntimeError('unexpected_tool_call')
                    loaded.append(name)
                    history.append({'type':'function_call_output','call_id':item['call_id'],'output':fixtures[name]})
            else: raise RuntimeError('tool_loop_limit')
            common.write_json(folder/'result.json',report)
        records=common.rows(proxy.state/'ledger.jsonl')
        responses=[r for r in records if r['kind']=='response']
        adopted=sum(r.get('inline_checkpoints_adopted',0) for r in responses)
        report['native_compactions']=response_compactions
        report['astral_adopted_checkpoints']=adopted
        report['projection_epochs']=[{k:r.get(k) for k in ('epoch','cut','projection_sha256','inline_scheduled','inline_checkpoints_adopted','effective_input_sha256','bytes_in','bytes_out','first_byte_ms','elapsed_ms','reason')} for r in responses]
        report['checks']={
            'both_fixtures_loaded':set(loaded)=={'config','audit'},
            'task_correct':report['turns'][-1].get('correct',False),
            'all_generations_completed':len(responses)==len(report['calls']) and all(r['outcome']=='completed' for r in responses),
            'all_usage_reported':all(isinstance(r.get('usage'),dict) for r in responses),
            'rollovers':response_compactions>=2 if arm!='full' else response_compactions==0,
            'no_plaintext_canaries':report['canaries_absent_from_final_upstream_input'] if arm!='full' else True,
        }
        if arm=='astral':
            report['checks'].update(
                astral_owns_checkpoints=adopted>=2,
                all_generations_committed=all(r.get('committed') for r in responses),
                final_projected_input_verified=responses[-1]['effective_input_sha256']==expected_final_hash,
                projection_reused_after_restart=bool(before_restart) and before_restart[0]['cut']>0 and before_restart==common.snapshot(proxy.state),
                no_history_resets=all(r['reason'] not in ('history_reset','contract_reset') for r in responses))
        report['passed']=all(report['checks'].values())
    except (OSError,ValueError,KeyError,RuntimeError) as error:
        report['error']=str(error)
    finally:
        proxy.close()
        report['seconds']=round(time.monotonic()-started,3)
        report['proxy_ingress']=proxy.request_counts
        report['proxy_exit_codes']=proxy.exit_codes
        report['usage']=common.summarize_ledger(proxy.state/'ledger.jsonl')
        if any(code!=0 for code in proxy.exit_codes): report.update(passed=False,error='proxy_shutdown_failed')
        common.write_json(folder/'result.json',report)
    return report


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--model',required=True)
    parser.add_argument('--upstream',required=True)
    parser.add_argument('--auth',choices=('gateway','api-key'),required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--binary',type=Path,default=common.ROOT/'target/release/ostk-gpt-cache')
    parser.add_argument('--seeds',default='731,947')
    parser.add_argument('--arms',default='full,provider,astral')
    parser.add_argument('--threshold',type=int,default=8192)
    parser.add_argument('--roll-bytes',type=int,default=64000)
    parser.add_argument('--timeout',type=int,default=90)
    parser.add_argument('--cache-mode',choices=('provider-default','implicit'),default='provider-default')
    args=parser.parse_args()
    args.seeds=[int(v) for v in args.seeds.split(',')]
    args.arms=args.arms.split(',')
    if not args.seeds or not args.arms or any(a not in ('full','provider','astral') for a in args.arms): parser.error('invalid seeds or arms')
    if min(args.threshold,args.roll_bytes,args.timeout)<1: parser.error('limits must be positive')
    modern=any(args.model==family or args.model.startswith(family+'-') for family in ('gpt-5.6','gpt-6-astra'))
    if urlsplit(args.upstream).hostname=='api.openai.com' and modern and args.cache_mode!='implicit':
        parser.error('use --cache-mode implicit so modern Platform cache controls match across all arms')
    if args.auth=='api-key' and not os.environ.get('OPENAI_API_KEY'): parser.error('OPENAI_API_KEY is required')
    args.binary,args.output=args.binary.resolve(),args.output.resolve()
    args.output.mkdir(parents=True,mode=0o700,exist_ok=False)
    report={'date':datetime.now(timezone.utc).date().isoformat(),'model':args.model,'upstream':args.upstream,'auth':args.auth,
            'seeds':args.seeds,'cache_mode':args.cache_mode,'threshold':args.threshold,'roll_bytes':args.roll_bytes,
            'ordering':[],'trials':[],'passed':False,
            'limits':'Synthetic tool-heavy task; small sample. Reported token use and wall time are not verified charges. Inline compaction work is not separately itemized. Separate stable keys and instruction nonces per arm.'}
    for seed in args.seeds:
        order=args.arms.copy()
        random.Random(seed).shuffle(order)
        report['ordering'].append({'seed':seed,'arms':order})
        for arm in order:
            folder=args.output/f'{seed}-{arm}'
            folder.mkdir(mode=0o700)
            result=run_arm(args,arm,seed,folder)
            report['trials'].append(result)
            common.write_json(args.output/'report.json',report)
            if 'error' in result:
                print(json.dumps({'blocked':result['error'],'arm':arm}),flush=True)
                return 1
    report['passed']=all(t['passed'] for t in report['trials'])
    common.write_json(args.output/'report.json',report)
    print(json.dumps({'report':str(args.output/'report.json'),'passed':report['passed']}),flush=True)
    return 0 if report['passed'] else 1


if __name__=='__main__':
    sys.exit(main())
