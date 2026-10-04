#!/usr/bin/env python3
"""Native-engine agreement is mandatory before typed gold publication."""
import sys
sys.dont_write_bytecode = True
import json,math,argparse
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
def normal(v):
 return float(v['decimal']) if isinstance(v,dict) and 'decimal' in v else v['temporal'] if isinstance(v,dict) else v
def same(a,b):
 return math.isclose(a,b,abs_tol=1e-7,rel_tol=1e-9) if isinstance(a,(float,int)) and isinstance(b,(float,int)) else a==b
parser=argparse.ArgumentParser();parser.add_argument('--verify-only',action='store_true');options=parser.parse_args()
report=json.loads((ROOT/'source/corrected-reference-executions.json').read_text());checks=[]
for c in report['cases']:
 pg=c['postgresql'];sq=c['sqlite'];assert 'error' not in pg and 'error' not in sq,c
 left=[[normal(v) for v in row] for row in sq['rows']];right=[[normal(v) for v in row] for row in pg['rows']];remaining=list(right)
 for row in left:
  ix=next((i for i,r in enumerate(remaining) if len(row)==len(r) and all(same(a,b) for a,b in zip(row,r))),None)
  assert ix is not None,(c['question_id'],row,remaining)
  remaining.pop(ix)
 assert not remaining,c['question_id']
 if c['question_id'] in [988,1011]:assert len(left)==len(right) and all(all(same(a,b) for a,b in zip(x,y)) for x,y in zip(left,right)),c['question_id']
 cols=[]
 for col in pg['columns']:
  typ={20:'int64',23:'int32',21:'int16',25:'utf8',1043:'utf8',701:'float64',700:'float32',1082:'date32',16:'boolean'}[col['oid']]
  item={'name':col['name'],'type':typ}
  if typ.startswith('float'):item['tolerance']={'absolute':1e-7,'relative':1e-9}
  cols.append(item)
 rows=[[str(v) if v is not None and col['type'].startswith('int') else v for col,v in zip(cols,row)] for row in right]
 gold_path=ROOT/'expected'/f"bird.formula1.{c['question_id']}.json"
 if options.verify_only:
  gold=json.loads(gold_path.read_text());assert [x['type'] for x in gold['columns']]==[x['type'] for x in cols],c['question_id']
  expected=[[int(v) if v is not None and col['type'].startswith('int') else v for col,v in zip(gold['columns'],row)] for row in gold['rows']];unmatched=list(right)
  for row in expected:
   ix=next((i for i,r in enumerate(unmatched) if len(row)==len(r) and all(same(a,b) for a,b in zip(row,r))),None);assert ix is not None,(c['question_id'],row);unmatched.pop(ix)
  assert not unmatched,c['question_id']
  if c['question_id'] in [988,1011]:assert expected==right,c['question_id']
 else:gold_path.write_text(json.dumps({'outcome':'result','columns':cols,'rows':rows},ensure_ascii=False,indent=2)+'\n')
 checks.append({'question_id':c['question_id'],'row_count':len(rows),'native_reference_agreement':True})
(ROOT/'source/oracle-review.json').write_text(json.dumps({'method':'Corrected native SQLite vs independent native PostgreSQL multiset agreement. Exact strings/integers; measurement floating tolerance absolute1e-7 relative1e-9. No product output.','cases':checks},indent=2)+'\n')
print('Verified frozen' if options.verify_only else 'Published',len(checks),'independently crosschecked typed golds')
