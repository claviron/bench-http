done = function(summary, latency, requests)
  io.write(string.format(
    '\nBENCH_HTTP_RESULT {"duration_us":%.0f,"requests":%.0f,' ..
    '"p99_us":%.0f,"errors":{"connect":%.0f,"read":%.0f,' ..
    '"write":%.0f,"status":%.0f,"timeout":%.0f}}\n',
    summary.duration, summary.requests, latency:percentile(99),
    summary.errors.connect, summary.errors.read, summary.errors.write,
    summary.errors.status, summary.errors.timeout))
end
