#!/usr/bin/env python3
"""Faithful full-family BIRD importer. Run with uv run --with sqlglot.
Gold rows are authored by reference_oracle.py, never Semantic DB output.
"""
import sys
sys.dont_write_bytecode = True
import csv,hashlib,json,re,sqlite3
from pathlib import Path
import sqlglot
from sqlglot import exp
from adjudications import corrections, REASONS, portable_sql
ROOT=Path(__file__).resolve().parents[1]
CLOCK='2026-10-04T00:00:00Z'
effective_questions={978:"How many distinct circuit venues are located in Austria, treating a venue as a distinct (location, latitude, longitude) combination? Return the total venue count followed by each venue's location, latitude and longitude, repeating the total count on each row."}
CSV_TABLES={'circuits','constructors','drivers','seasons','status'}
def write(path,data): (ROOT/path).write_text(json.dumps(data,ensure_ascii=False,indent=2)+'\n')
def extension(data):return {'vendor_name':'SEMANTIC_DB','data':json.dumps(data,separators=(',',':'))}
def expression(sql):return {'dialects':[{'dialect':'ANSI_SQL','expression':sql}]}
def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
db=sqlite3.connect(ROOT/'source/formula_1.sqlite')
pg_all=json.loads((ROOT/'source/mini_dev_pg.json').read_text());sq_all=json.loads((ROOT/'source/mini_dev_sqlite.json').read_text())
pg=[r for r in pg_all if r['db_id']=='formula_1'];sq={r['question_id']:r for r in sq_all if r['db_id']=='formula_1'}
assert len(pg)==66 and {r['question_id'] for r in pg}==set(sq)
write('source/selected_original.json',[{'question_id':r['question_id'],'question':r['question'],'evidence':r['evidence'],'difficulty':r['difficulty'],'original_postgresql_sql':r['SQL'],'original_sqlite_question':sq[r['question_id']]['question'],'original_sqlite_evidence':sq[r['question_id']]['evidence'],'original_sqlite_sql':sq[r['question_id']]['SQL']} for r in pg])
original_tables=[r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
schemas={};datasets=[];relationships=[];identity=[];sources={};ddl=['BEGIN;','DROP SCHEMA IF EXISTS bird_formula1 CASCADE;','CREATE SCHEMA bird_formula1;'];fixture=[]
logical={'int64':'Integer','float64':'Float','utf8':'String','date32':'Date'}
physical={'int64':'Int64','float64':'Float64','utf8':'Utf8','date32':'Date32'}
pg_type={'int64':'bigint','float64':'double precision','utf8':'text','date32':'date'}
for original in original_tables:
 name=original.lower();cols=list(db.execute('PRAGMA table_info("'+original+'")'));rows=db.execute('SELECT * FROM "'+original+'"').fetchall();keys=[c[1].lower() for c in sorted(cols,key=lambda c:c[5]) if c[5]]
 fields=[];schema_cols=[]
 description_file=ROOT/'source/database_description'/f'{original}.csv';descriptions={}
 if description_file.exists():
  for d in csv.DictReader(description_file.read_text(encoding='utf-8-sig',errors='replace').splitlines()):
   k=d.get('original_column_name') or d.get('column_name') or next(iter(d.values()),'');descriptions[k.strip().lower()]='; '.join(str(v) for v in d.values() if v)
 for c in cols:
  cname=c[1].lower();typ='int64' if 'INT' in c[2].upper() else 'float64' if c[2].upper() in ['REAL','FLOAT','DOUBLE'] else 'date32' if c[2].upper()=='DATE' else 'utf8';nullable=not bool(c[3] or c[5]);observed=any(row[c[0]] is None for row in rows)
  assert nullable or not observed,(name,cname,'original declared nonnull violated')
  schema_cols.append({'name':cname,'type':typ,'nullable':nullable,'physical_nullable':nullable})
  description=descriptions.get(cname,f'Original benchmark column {original}.{c[1]}, declared {c[2]}.')
  f={'name':cname,'datatype':logical[typ],'description':description,'ai_context':{'synonyms':[c[1]]},'expression':expression(cname)}
  if cname=='milliseconds':f['custom_extensions']=[extension({'kind':'unit','unit':{'kind':'named','id':'millisecond'}})]
  fields.append(f)
 schemas[name]={'source':'csv' if name in CSV_TABLES else 'postgres','row_count':len(rows),'primary_key':keys,'columns':schema_cols}
 with (ROOT/'data'/f'{name}.csv').open('w',newline='') as out:
  writer=csv.writer(out);writer.writerow([c[1].lower() for c in cols]);assert not any(v=='\\N' for row in rows for v in row if isinstance(v,str));writer.writerows([['\\N' if v is None else v for v in row] for row in rows])
 datasets.append({'name':name,'source':('csv.' if name in CSV_TABLES else 'facts.')+name,'primary_key':keys,'description':f'Complete original BIRD Formula 1 table {original}; {len(rows)} source records. No subsampling or synthetic replacement.','ai_context':{'synonyms':[original],'instructions':'Preserve original row multiplicity; missing values are unknown. Fields such as result rank, finishing position, championship standing, and qualifying position are distinct.'},'fields':fields})
 if keys:identity.append(extension({'kind':'entity_identity','dataset':name,'id':'bird/formula1/'+name,'keys':keys}))
 if name in CSV_TABLES:sources['csv.'+name]={'connection':'csv','path':'data/'+name+'.csv','null_regex':'^\\\\N$','non_nullable_columns':[c['name'] for c in schema_cols if not c['nullable']],'physical_types':{c['name']:physical[c['type']] for c in schema_cols}}
 else:
  sources['facts.'+name]={'connection':'facts','schema':'bird_formula1','table':name}
  columns=[f'"{c["name"]}" {pg_type[c["type"]]}'+('' if c['nullable'] else ' NOT NULL') for c in schema_cols];columns.append('PRIMARY KEY ('+','.join('"'+k+'"' for k in keys)+')') if keys else None
  ddl.append('CREATE TABLE bird_formula1.'+name+' ('+', '.join(columns)+');');ddl.append("\\copy bird_formula1."+name+" FROM '/data/"+name+".csv' WITH (FORMAT csv, HEADER true, NULL '\\N');")
  ddl.append('DO $$ BEGIN IF (SELECT count(*) FROM bird_formula1.'+name+') <> '+str(len(rows))+" THEN RAISE EXCEPTION 'fixture count mismatch: "+name+"'; END IF; END $$;")
 for fk in db.execute('PRAGMA foreign_key_list("'+original+'")'):
  relationships.append({'name':name+'_'+fk[3].lower()+'_'+fk[2].lower(),'from':name,'to':fk[2].lower(),'from_columns':[fk[3].lower()],'to_columns':[fk[4].lower()],'ai_context':{'instructions':f'Original declared foreign key {original}.{fk[3]} -> {fk[2]}.{fk[4]}; exact equality, null keys do not match.'}})
 write('expected/fixture-'+name+'.json',{'outcome':'result','columns':[{'name':'row_count','type':'int64'}],'rows':[[str(len(rows))]]});fixture.append({'sql':'SELECT COUNT(*) AS row_count FROM '+name,'expected':'expected/fixture-'+name+'.json'})
assert len(schemas)==13 and sum(s['row_count'] for s in schemas.values())==493257
write('schemas.json',schemas)
# Upstream evidence is legitimate benchmark input, separate from SQL and gold.
knowledge=sorted(set((sq[r['question_id']]['evidence'] if r['question_id'] in [964,988] else r['evidence']) for r in pg if r.get('evidence')))
model={'version':'0.2.0.dev0','semantic_model':[{'name':'bird_formula1','description':'Complete external BIRD Mini-Dev Formula 1 family, independent evaluation suite. Original questions and data unchanged.','ai_context':{'instructions':'This benchmark uses the full historic Formula1 source snapshot, and fixed request clock '+CLOCK+'. Reference names are driverRef/circuitRef/constructorRef. Rank, position, positionOrder, championship standings, and qualifying position are distinct; use the question and provided official knowledge. Preserve table grain and row multiplicity; do not default to DISTINCT. Race ranked first/second and champion refer to finishing positionOrder unless fastest-lap rank is explicitly requested. First participation means earliest available race date, then round/raceId; youngest means latest known DOB with driverId tie break. Best observed timing excludes missing timings. Driver counts/lists use driver identity rather than repeated laps or results. Geographic venue identity is (location,lat,lng), which can have multiple historical circuitIDs. Per-circuit lap records use minimum observed nonnull duration, retaining circuit identity; missing observations do not establish a record. Original timing fields are strings in seconds, minutes:seconds or hours:minutes:seconds and all components must be parsed explicitly when requested; seconds and subordinate minutes are in [0,60). Rank minimum durations with driverId tie breaks. These are authored task interpretation policies, not expected results. Here is official benchmark knowledge (not reference SQL or expected answers):\n'+'\n'.join(knowledge)},'datasets':datasets,'relationships':relationships,'custom_extensions':identity}]}
write('model.ossie.yaml',model)
write('semantic-db.yaml',{'ossie':'model.ossie.yaml','connections':{'csv':{'connector':'csv'},'facts':{'connector':'postgres','connection_string_env':'BIRD_DATABASE_URL','allow_insecure_transport':True}},'sources':sources})
# SQLGlot only normalizes identifiers and frozen request-clock functions initially.
# Additional adjudicated dialect equivalences are explicit in transform_sql below.
def transform_sql(sql):
 tree=sqlglot.parse_one(sql,read='postgres')
 for node in tree.walk():
  if isinstance(node,exp.Identifier):node.set('this',node.this.lower());node.set('quoted',False)
  if isinstance(node,(exp.CurrentTimestamp,exp.CurrentDate)):
   node.replace(exp.Cast(this=exp.Literal.string(CLOCK.replace('T',' ').replace('Z','')),to=exp.DataType.build('TIMESTAMP' if isinstance(node,exp.CurrentTimestamp) else 'DATE')))
 result=tree.sql(dialect='postgres')
 return result
cases=[]
for r in pg:
 id='bird.formula1.'+str(r['question_id']);sql=transform_sql(portable_sql(r['question_id'],corrections('postgres').get(r['question_id'],r['SQL']),'postgres'));tags=['external','bird','formula1',r['difficulty']]
 referenced={t.name.lower() for t in sqlglot.parse_one(sql,read='postgres').find_all(exp.Table) if t.name.lower() in schemas};cross=bool(referenced&CSV_TABLES and referenced-set(CSV_TABLES));tags+=['cross_source'] if cross else []
 if r['question_id'] in [880,895,960]:
  sql='SELECT CAST(value AS DOUBLE PRECISION) AS value FROM ('+sql+') AS measured(value)'
 if r['question_id']==898:
  sql=sql.replace('EXTRACT(YEAR FROM CAST(\'2026-10-04 00:00:00\' AS TIMESTAMP)) - EXTRACT(YEAR FROM dob) AS age','CAST(EXTRACT(YEAR FROM CAST(\'2026-10-04 00:00:00\' AS TIMESTAMP)) - EXTRACT(YEAR FROM dob) AS BIGINT) AS age')
 cases.append({'id':id,'primary_group':r['difficulty'],'tags':tags,'question':effective_questions.get(r['question_id'],r['question']),'sql':sql,'expected':'expected/'+id+'.json','comparison':{'ordered':r['question_id'] in [988,1011],'assert_names':False,'assert_physical_types':False},'requirements':['Complete family selection: original question_id '+str(r['question_id'])+'; no SQL feature exclusions.','Original question/evidence/PostgreSQL SQL/SQLite SQL retained in source/selected_original.json.','Result oracle executes original SQLite SQL and independent PostgreSQL reference; any dialect discrepancy is explicitly adjudicated in source/oracle-review.json.'],'oracle_reasoning':'Original database reference execution; never product output.','distinguishing_rows':['All original '+str(sum(s['row_count'] for s in schemas.values()))+' database rows retained. Source snapshot and query ID identify independently auditable upstream counterexamples.']})
write('cases.json',cases)
write('source/question-errata.json',[{'question_id':978,'original_question':next(r['question'] for r in pg if r['question_id']==978),'effective_question':effective_questions[978],'reason':'Visible clarification of previously independently reviewed geographic venue counting grain and repeated-total output shape; original held-times phrasing is ambiguous. No SQL, gold, data or tolerance changes.','review_disposition':'Independent reviewer approved; derived variant, not official BIRD leaderboard wording'}])
write('source/adjudications.json',[{'question_id':r['question_id'],'reason':REASONS.get(r['question_id'],'Mechanical identifier/clock normalization; upstream semantics preserved.'),'corrected_sqlite_sql':portable_sql(r['question_id'],corrections('sqlite').get(r['question_id'],sq[r['question_id']]['SQL']),'sqlite'),'adapted_postgresql_sql':cases[i]['sql'],'review_disposition':'Independent reviewer accepted task resolution' if r['question_id'] in REASONS else 'Unchanged reference task'} for i,r in enumerate(pg)])
write('manifest.json',{'format_version':1,'id':'bird-minidev-formula1-v1','version':'1.0.3','project':'semantic-db.yaml','cases':'cases.json','schemas':'schemas.json','execution':{'max_requests':1024},'required_paired_cases':66,'required_companion_cases':0,'context':{'reference_time':CLOCK,'timezone':'UTC'},'environment':{'compose_files':['compose.yaml'],'services':['postgres'],'bootstrap_service':'bootstrap','startup_timeout_seconds':120,'bootstrap_timeout_seconds':180,'bindings':{'BIRD_DATABASE_URL':{'service':'postgres','port':5432,'template':'postgres://bird:bird@{host}:{port}/bird?sslmode=disable'}}},'fixtures':fixture,'public_cases':['bird.formula1.945','bird.formula1.859','bird.formula1.901']})
(ROOT/'bootstrap/seed.sql').write_text('\n'.join(ddl+['COMMIT;'])+'\n')
(ROOT/'bootstrap/data.sha256').write_text(''.join(sha(f)+'  /data/'+f.name+'\n' for f in sorted((ROOT/'data').iterdir()) if f.is_file()))
write('source/selection.json',{'selection_rule':'ALL canonical PostgreSQL Mini-Dev records where db_id == formula_1','question_ids':[r['question_id'] for r in pg],'counts_by_difficulty':{x:sum(r['difficulty']==x for r in pg) for x in ['simple','moderate','challenging']},'original_table_count':13,'original_row_count':493257,'cross_source_cases':sum('cross_source' in c['tags'] for c in cases),'clock':CLOCK,'transforms':'Identifier lowercase only (case-insensitive original SQL names); equivalent NOTNULL syntax serialization; CURRENT_TIMESTAMP/CURRENT_DATE frozen to request clock. Original input bytes preserved.'})
print('Complete family:',len(cases),'cases;',sum(s['row_count'] for s in schemas.values()),'rows;',sum('cross_source' in c['tags'] for c in cases),'cross-source')

provenance=json.loads((ROOT/'source/provenance.json').read_text())
provenance['edition']='BIRD-derived corrected Formula1 variant; not an official leaderboard score'
provenance['changes']='Full family/data retained; explicit reviewed task-semantic annotation repairs; reversible identifier case folding; fixed request clock; measurement numeric transport casts. Both original language/evidence/SQL variants retained.'
provenance['source_files']={str(f.relative_to(ROOT)):sha(f) for f in sorted((ROOT/'source').rglob('*')) if f.is_file() and f.name!='provenance.json'}
write('source/provenance.json',provenance)
