import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import ts from 'typescript';
const findings=[];
const locales=['en-US','zh-CN'];
const resources={};
const resourceFiles=[];
function literal(node){
  while(ts.isAsExpression(node)||ts.isSatisfiesExpression(node)||ts.isParenthesizedExpression(node))node=node.expression;
  if(ts.isStringLiteral(node)||ts.isNoSubstitutionTemplateLiteral(node))return node.text;
  if(ts.isObjectLiteralExpression(node))return Object.fromEntries(node.properties.filter(ts.isPropertyAssignment).map(p=>[p.name.text,literal(p.initializer)]));
  return null;
}
function parse(file){return ts.createSourceFile(file,readFileSync(file,'utf8'),ts.ScriptTarget.Latest,true,file.endsWith('.tsx')?ts.ScriptKind.TSX:ts.ScriptKind.TS);}
function params(text){return [...new Set([...text.matchAll(/\{([a-zA-Z0-9_]+)\}/g)].map(m=>m[1]))].sort();}
for(const locale of locales){
  resources[locale]={};
  for(const file of readdirSync(`src/i18n/locales/${locale}`)){
    if(!file.endsWith('.ts'))continue;
    const path=`src/i18n/locales/${locale}/${file}`;resourceFiles.push(path);
    const source=parse(path);const declaration=source.statements.find(ts.isExportAssignment);
    function checkDuplicates(node){
      if(ts.isObjectLiteralExpression(node)){
        const names=new Set();
        for(const property of node.properties.filter(ts.isPropertyAssignment)){
          const key=property.name.text;
          if(names.has(key))findings.push(`${path}: duplicate object key ${key}`);
          names.add(key);
        }
      }
      ts.forEachChild(node,checkDuplicates);
    }
    checkDuplicates(source);
    const values=declaration?literal(declaration.expression):null;
    if(!values||typeof values!=='object'){findings.push(`${path}: resource must default-export a static object`);continue;}
    for(const [key,value]of Object.entries(values)){
      if(typeof value!=='string'||!key.startsWith(`${file.slice(0,-3)}.`))findings.push(`${path}: invalid domain key ${key}`);
      if(key in resources[locale])findings.push(`${path}: duplicate key ${key}`);
      resources[locale][key]=value;
    }
  }
}
for(const key of new Set([...Object.keys(resources['en-US']),...Object.keys(resources['zh-CN'])])){
  const en=resources['en-US'][key],zh=resources['zh-CN'][key];
  if(typeof en!=='string'||typeof zh!=='string')findings.push(`catalog: missing language key ${key}`);
  else if(JSON.stringify(params(en))!==JSON.stringify(params(zh)))findings.push(`catalog: interpolation mismatch ${key}`);
}
const manifest=readFileSync('src/i18n/catalogs.ts','utf8');
for(const path of resourceFiles)if(!manifest.includes(`'./${path.slice('src/i18n/'.length,-3)}'`))findings.push(`host loader: resource is not imported ${path}`);
const remoteManifest=readFileSync('src/i18n/remoteCatalogs.ts','utf8');
for(const locale of locales)if(!remoteManifest.includes(`'./locales/${locale}/remote'`))findings.push(`remote loader: missing ${locale}`);
function checkLoaderBindings(file,remoteOnly=false){
  const source=parse(file),bindings=new Map();
  for(const node of source.statements){
    if(ts.isImportDeclaration(node)&&ts.isStringLiteral(node.moduleSpecifier)&&node.importClause?.name){
      const match=/^\.\/locales\/(en-US|zh-CN)\/([\w-]+)$/.exec(node.moduleSpecifier.text);
      if(match)bindings.set(node.importClause.name.text,{locale:match[1],domain:match[2]});
    }
  }
  const used=new Set();
  function visit(node){
    if(ts.isPropertyAssignment(node)&&locales.includes(node.name.text)){
      const locale=node.name.text;
      const names=ts.isObjectLiteralExpression(node.initializer)?node.initializer.properties.filter(ts.isSpreadAssignment).map(item=>item.expression.getText(source)):[node.initializer.getText(source)];
      for(const name of names){const binding=bindings.get(name);if(!binding||binding.locale!==locale||(remoteOnly&&binding.domain!=='remote'))findings.push(`${file}: invalid ${locale} resource binding ${name}`);else used.add(name);}
    }
    ts.forEachChild(node,visit);
  }
  visit(source);
  for(const name of bindings.keys())if(!used.has(name))findings.push(`${file}: imported resource omitted from exported catalog ${name}`);
}
checkLoaderBindings('src/i18n/catalogs.ts');
checkLoaderBindings('src/i18n/remoteCatalogs.ts',true);
const files=[];
function walk(dir){for(const entry of readdirSync(dir)){const path=join(dir,entry);if(statSync(path).isDirectory())walk(path);else if(/\.(tsx?|js)$/.test(path))files.push(path);}}
walk('src');walk('bundled-modules');
const attributeNames=new Set(['aria-label','aria-description','placeholder','title','label','alt']);
for(const file of files){
  if(file.startsWith('src/i18n/locales/'))continue;
  const source=parse(file);const remote=file.startsWith('src/lan/remote/');
  function visit(node){
    const at=()=>`${file}:${source.getLineAndCharacterOfPosition(node.getStart(source)).line+1}`;
    if(ts.isCallExpression(node)&&node.arguments[0]&&ts.isObjectLiteralExpression(node.arguments[0])){
      const callee=node.expression.getText(source);
      if(/(?:Modal\.(?:confirm|alert|error)|confirmAction|^confirm$)/.test(callee)){
        for(const property of node.arguments[0].properties.filter(ts.isPropertyAssignment)){
          if(!['title','content','confirmText','cancelText','okText'].includes(property.name.text))continue;
          const text=literal(property.initializer);
          if(typeof text==='string'&&/[\p{L}]/u.test(text))findings.push(`${at()}: raw dialog copy ${property.name.text}=${text}`);
        }
      }
    }
    if(ts.isCallExpression(node)&&((ts.isIdentifier(node.expression)&&['t','translate'].includes(node.expression.text))||(ts.isPropertyAccessExpression(node.expression)&&node.expression.name.text==='t'))){
      const argument=node.arguments[ts.isIdentifier(node.expression)&&node.expression.text==='translate'?1:0];
      const key=argument&&literal(argument);
      if(typeof key==='string'){
        if(!(key in resources['en-US']))findings.push(`${at()}: unknown key ${key}`);
        else{
          if(remote&&!key.startsWith('remote.'))findings.push(`${at()}: remote resource outside remote domain ${key}`);
          const expected=params(resources['en-US'][key]);
          const variables=node.arguments[ts.isIdentifier(node.expression)&&node.expression.text==='translate'?2:1];
          if(expected.length&&(!variables||!ts.isObjectLiteralExpression(variables)))findings.push(`${at()}: interpolation object required for ${key}`);
          if(variables&&ts.isObjectLiteralExpression(variables)){
            const names=variables.properties.filter(p=>ts.isPropertyAssignment(p)||ts.isShorthandPropertyAssignment(p)).map(p=>p.name.text).sort();
            if(expected.some(name=>!names.includes(name)))findings.push(`${at()}: missing interpolation for ${key}: ${expected.filter(n=>!names.includes(n)).join(',')}`);
          }
        }
      }
    }
    if(ts.isJsxText(node)){
      const text=node.getText(source).replace(/\s+/g,' ').trim();
      if(text&&/[\p{L}]/u.test(text))findings.push(`${at()}: raw UI text ${text}`);
    }
    if(ts.isJsxAttribute(node)&&attributeNames.has(node.name.getText(source))&&node.initializer){
      const text=literal(node.initializer);
      if(typeof text==='string'&&/[\p{L}]/u.test(text)&&!(/^(?:https?:\/\/|\/|Python$|Node\.js$|SHA-256:|Bearer$|x-api-key$|model-id$|tokens$|ms$|V1\s*·\s*D021$)/i.test(text)))findings.push(`${at()}: raw attribute ${node.name.getText(source)}=${text}`);
    }
    ts.forEachChild(node,visit);
  }
  visit(source);
}
// Module resources stay private, but both languages must expose matching keys and interpolation parameters.
for(const file of files.filter(f=>f.startsWith('bundled-modules/'))){
  const source=parse(file);
  function visit(node){
    if(ts.isVariableDeclaration(node)&&node.name.getText(source)==='catalogs'&&node.initializer){
      const catalog=literal(node.initializer);
      if(catalog&&catalog['en-US']&&catalog['zh-CN']){
        for(const key of new Set([...Object.keys(catalog['en-US']),...Object.keys(catalog['zh-CN'])])){
          const en=catalog['en-US'][key],zh=catalog['zh-CN'][key];
          if(typeof en!=='string'||typeof zh!=='string'||JSON.stringify(params(en))!==JSON.stringify(params(zh)))findings.push(`${file}: module resource mismatch ${key}`);
        }
      }
    }
    ts.forEachChild(node,visit);
  }visit(source);
}
console.log(`i18n: ${Object.keys(resources['en-US']).length} keys, ${resourceFiles.length} language-domain resources, ${files.length} source files.`);
if(findings.length){console.error(findings.join('\n'));process.exitCode=1;}else console.log('Language keys, interpolation parameters, loader coverage and static callers passed.');
