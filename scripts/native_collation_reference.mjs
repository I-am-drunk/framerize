import fs from 'node:fs';

const cases = JSON.parse(fs.readFileSync(0, 'utf8'));
const rows = cases.map(({locale, strings}) => {
  const collator = new Intl.Collator(locale);
  const texts = strings.map(units => String.fromCharCode(...units));
  const signs = texts.flatMap(left => texts.map(right => Math.sign(collator.compare(left, right))));
  return {locale, signs, options: collator.resolvedOptions()};
});
process.stdout.write(JSON.stringify({versions: process.versions, rows,
  defaultLocale: new Intl.Collator().resolvedOptions().locale}));
