'use strict'

const path = require('path')
const express = require('express')
const { OidcExchange } = require('@oidc-exchange/node')

const oidc = new OidcExchange({
  config: path.resolve(__dirname, '..', 'config.toml'),
})

const app = express()

app.all('/auth/*', (req, res) => {
  const chunks = []

  req.on('data', (chunk) => chunks.push(chunk))
  req.on('end', () => {
    const body = chunks.length > 0 ? Buffer.concat(chunks) : undefined

    const headers = []
    const raw = req.rawHeaders
    for (let i = 0; i < raw.length; i += 2) {
      headers.push({ name: raw[i], value: raw[i + 1] })
    }

    const oidcPath = req.originalUrl.replace(/^\/auth/, '')

    const response = oidc.handleRequest({
      method: req.method,
      path: oidcPath || '/',
      headers,
      body,
    })

    for (const { name, value } of response.headers) {
      res.setHeader(name, value)
    }

    res.status(response.status).end(response.body)
  })
})

const port = process.env.PORT || 8080

app.listen(port, () => {
  console.log(`OIDC-Exchange (Express) listening on http://localhost:${port}`)
})
