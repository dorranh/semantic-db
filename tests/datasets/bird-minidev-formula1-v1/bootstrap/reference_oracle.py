#!/usr/bin/env python3
"""Independent original SQLite + native PostgreSQL audit, no product calls.
Run with uv run --with sqlglot --with 'psycopg[binary]' python ...
Uses an isolated temporary PostgreSQL container and always tears it down.
"""
import sys
sys.dont_write_bytecode = True
import argparse,csv,datetime,hashlib,json,os,shlex,sqlite3,subprocess,time
from decimal import Decimal
from pathlib import Path
import psycopg
import author_dataset as authored
from adjudications import corrections, portable_sql
ROOT=authored.ROOT

def docker(*args,check=True):
 return subprocess.run(['sg','docker','-c',shlex.join(['docker',*args])],check=check,text=True,capture_output=True)
def encode(v):
 if isinstance(v,Decimal):return {'decimal':str(v)}
 if isinstance(v,(datetime.date,datetime.datetime,datetime.time)):return {'temporal':v.isoformat()}
 if isinstance(v,datetime.timedelta):return {'interval_microseconds':str(v.days*86400000000+v.seconds*1000000+v.microseconds)}
 return v

def main():
 parser=argparse.ArgumentParser();parser.add_argument('--report',default='source/reference-executions.json');opts=parser.parse_args()
 name=f'semantic-bird-reference-{os.getpid()}-{time.time_ns()%1000000}'
 report={'source_sqlite_sha256':authored.sha(ROOT/'source/formula_1.sqlite'),'clock':authored.CLOCK,'method':'Original SQLite engine and independent native PostgreSQL 16.6; every original record loaded. This is an oracle, not the mixed-source product test.','cases':[]}
 db=sqlite3.connect(ROOT/'source/formula_1.sqlite');delegate=sqlite3.connect(':memory:')
 # Original SQL remains byte-identical; clock functions receive fixed host context.
 clock=authored.CLOCK.replace('T',' ').replace('Z','')
 for fn in ['strftime','date','datetime','julianday']:
  def fixed(*args,func=fn):
   args=tuple(clock if a=='now' else a for a in args)
   return delegate.execute('SELECT '+func+'('+','.join('?' for _ in args)+')',args).fetchone()[0]
  db.create_function(fn,-1,fixed)
 originals=json.loads((ROOT/'source/selected_original.json').read_text());schemas=json.loads((ROOT/'schemas.json').read_text());cases={int(c['id'].rsplit('.',1)[1]):c for c in json.loads((ROOT/'cases.json').read_text())}
 started=False
 try:
  docker('run','--detach','--name',name,'--publish','127.0.0.1::5432','--env','POSTGRES_USER=bird','--env','POSTGRES_PASSWORD=bird','--env','POSTGRES_DB=bird','postgres:16.6-bookworm');started=True
  ready=False
  for _ in range(60):
   if docker('exec',name,'pg_isready','-h','127.0.0.1','-U','bird','-d','bird',check=False).returncode==0:ready=True;break
   time.sleep(1)
  if not ready:raise RuntimeError('reference PostgreSQL readiness timeout')
  endpoint=docker('port',name,'5432/tcp').stdout.strip();port=int(endpoint.rsplit(':',1)[1])
  with psycopg.connect(host='127.0.0.1',port=port,user='bird',password='bird',dbname='bird',sslmode='disable',autocommit=True) as pg:
   pg.execute("SET timezone='UTC'");pg.execute("SET statement_timeout='30s'")
   types={'int64':'bigint','float64':'double precision','utf8':'text','date32':'date'}
   for n,schema in schemas.items():
    cols=[f'"{c["name"]}" {types[c["type"]]}'+('' if c['nullable'] else ' NOT NULL') for c in schema['columns']]
    if schema['primary_key']:cols.append('PRIMARY KEY ('+','.join('"'+k+'"' for k in schema['primary_key'])+')')
    pg.execute('CREATE TABLE "'+n+'" ('+', '.join(cols)+')')
    with pg.cursor().copy('COPY "'+n+'" FROM STDIN WITH (FORMAT csv, HEADER true, NULL \'\\N\')') as copy:
     with (ROOT/'data'/f'{n}.csv').open('rb') as source:
      while block:=source.read(1<<20):copy.write(block)
    assert pg.execute('SELECT COUNT(*) FROM "'+n+'"').fetchone()[0]==schema['row_count']
   for original in originals:
    ident=original['question_id'];case={'question_id':ident};sql=cases[ident]['sql']
    try:
     cursor=db.execute(portable_sql(ident,corrections('sqlite').get(ident,original['original_sqlite_sql']),'sqlite'));out=cursor.fetchall();case['sqlite']={'columns':[x[0] for x in cursor.description],'rows':[[encode(v) for v in row] for row in out]}
    except Exception as e:case['sqlite']={'error':str(e)}
    try:
     cursor=pg.execute(sql);out=cursor.fetchall();case['postgresql']={'sql':sql,'columns':[{'name':x.name,'oid':x.type_code,'precision':x.precision,'scale':x.scale} for x in cursor.description],'rows':[[encode(v) for v in row] for row in out]}
    except Exception as e:case['postgresql']={'sql':sql,'error':str(e)}
    report['cases'].append(case);print(ident,'sqlite',len(case.get('sqlite',{}).get('rows',[])),'postgres',len(case.get('postgresql',{}).get('rows',[])),case.get('postgresql',{}).get('error',''),flush=True)
  (ROOT/opts.report).write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
 finally:
  if started:
   docker('container','stop','--time','5',name,check=False);docker('container','rm',name,check=False)
if __name__=='__main__':main()
