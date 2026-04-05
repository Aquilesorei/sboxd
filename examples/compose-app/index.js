const { Client } = require('pg');

async function main() {
  const client = new Client({
    host: process.env.DB_HOST || 'localhost',
    port: 5432,
    database: 'appdb',
    user: 'app',
    password: 'secret',
  });

  try {
    await client.connect();
    const res = await client.query('SELECT NOW() AS now');
    console.log('Connected to DB. Server time:', res.rows[0].now);
    await client.end();
  } catch (err) {
    console.error('DB connection failed (expected outside compose):', err.message);
  }
}

main();
