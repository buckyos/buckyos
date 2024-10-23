// 定义协议类型枚举
enum RPCProtocolType {
    HttpPostJson = 'HttpPostJson'
  }
  
  // 定义错误类型
  class RPCError extends Error {
    constructor(message: string) {
      super(message);
      this.name = 'RPCError';
    }
  }
  
  // RPC 客户端实现
  class kRPCClient {
    private client: typeof fetch;
    private serverUrl: string;
    private protocolType: RPCProtocolType;
    private seq: number;
    private sessionToken: string | null;
    private initToken: string | null;
  
    constructor(url: string, token?: string) {
      this.client = fetch;
      this.serverUrl = url;
      this.protocolType = RPCProtocolType.HttpPostJson;
      // 使用毫秒时间戳作为初始序列号
      this.seq = Date.now() * 1000;
      this.sessionToken = token || null;
      this.initToken = token || null;
    }
  
    // 公开的调用方法
    async call(method: string, params: any): Promise<any> {
      return this._call(method, params);
    }
  

    private async _call(method: string, params: any): Promise<any> {

      this.seq += 1;
      const currentSeq = this.seq;
  
      const requestBody = {
        method,
        params,
        sys: this.sessionToken ? 
          [currentSeq, this.sessionToken] : 
          [currentSeq]
      };
  
      try {
        const response = await this.client(this.serverUrl, {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json'
          },
          body: JSON.stringify(requestBody),
          signal: AbortSignal.timeout(15000)
        });
  
        if (!response.ok) {
          throw new RPCError(`RPC call error: ${response.status}`);
        }
  
        const rpcResponse = await response.json();
  

        if (rpcResponse.sys) {
          const sys = rpcResponse.sys;
          if (!Array.isArray(sys)) {
            throw new RPCError('sys is not array');
          }

          if (sys.length > 1) {
            const responseSeq = sys[0];
            if (typeof responseSeq !== 'number') {
              throw new RPCError('sys[0] is not number');
            }
            if (responseSeq !== currentSeq) {
              throw new RPCError(`seq not match: ${responseSeq}!=${currentSeq}`);
            }
          }
  
          if (sys.length > 2) {
            const token = sys[1];
            if (typeof token !== 'string') {
              throw new RPCError('sys[1] is not string');
            }
            this.sessionToken = token;
          }
        }
  
        if (rpcResponse.error) {
          throw new RPCError(`RPC call error: ${rpcResponse.error}`);
        }
  
        return rpcResponse.result;
      } catch (error) {
        if (error instanceof RPCError) {
          throw error;
        }
        throw new RPCError(`RPC call failed: ${error.message}`);
      }
    }
  }
  
  // 导出类和类型
  export { kRPCClient, RPCProtocolType, RPCError };