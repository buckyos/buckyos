import Koa from 'koa';
import Router from '@koa/router';
import bodyParser from '@koa/bodyparser';
import { authComponent } from './auth';
import { Storage } from './storage';

async function startServer() {
  const app = new Koa();
  const router = new Router();

  let storage = new Storage('server.db');
  await storage.init();

  app.use(bodyParser());

  app.use(authComponent(storage));

  // Define a simple route
  router.get('/', (ctx) => {
    ctx.body = 'Hello, World!';
  });

  router.post('/data', (ctx) => {
    ctx.body = {
        result: 1,
        request: ctx.request.body.content
    }
  });

  router.post('/version/url', async (ctx) => {
    const { version, os, arch, url } = ctx.request.body.content;
    if (!version || !os || !arch || !url) {
      ctx.status = 400;
      ctx.body = { error: 'Missing required fields' };
      return;
    }

    try {
      await storage.setVersionUrl(version, os, arch, url);
      ctx.body = { result: 1 };
    } catch (error) {
      ctx.status = 500;
      ctx.body = { error: 'Failed to set version URL' };
    }
  })

  router.post('/version/test', async (ctx) => {
    const { version, os, arch, tested } = ctx.request.body.content;
    if (!version || !os || !arch || typeof tested !== 'boolean') {
      ctx.status = 400;
      ctx.body = { error: 'Missing required fields' };
      return;
    }

    try {
      await storage.setVersionTestResult(version, os, arch, tested);
      ctx.body = { result: 1 };
    } catch (error) {
      ctx.status = 500;
      ctx.body = { error: 'Failed to set version test result' };
    }
  });

  router.post('/version/publish', async (ctx) => {
    const { version, os, arch, published } = ctx.request.body.content;
    if (!version || !os || !arch || typeof published !== 'boolean') {
      ctx.status = 400;
      ctx.body = { error: 'Missing required fields' };
      return;
    }

    try {
      await storage.setVersionPublishResult(version, os, arch, published);
      ctx.body = { result: 1 };
    } catch (error) {
      ctx.status = 500;
      ctx.body = { error: 'Failed to set version publish result' };
    }
  });

  router.get("/version", async (ctx) => {
    const query = ctx.request.query;
    let pageNum = parseInt(query.page as string) || 1;
    let pageSize = parseInt(query.size as string) || 0;

    let os;
    if (typeof query.os === 'string') {
      os = [query.os];
    } else {
        os = query.os;
    }

    let arch;
    if (typeof query.arch === 'string') {
      arch = [query.arch];
    } else {
        arch = query.arch;
    }


    let notest = query.notest === 'true' ? true : false;
    let nopub = query.nopub === 'true' ? true : false;

    let versions = await storage.getVersions(pageNum, pageSize, os, arch, notest, nopub);

    ctx.body = {
      items: versions,
      pageNum: pageNum,
      pageSize: pageSize,
    };
  });

  router.get("/version/total", async (ctx) => {
    ctx.body = {
        total: await storage.getVersionCount()
    }
  })

  app.use(router.routes()).use(router.allowedMethods());

  app.listen(9800, () => {
    console.log('Server is running on http://localhost:9800');
  });
}

startServer();
